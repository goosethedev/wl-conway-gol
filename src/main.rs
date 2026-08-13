mod gol;

use std::os::fd::AsFd;
use std::time::Duration;

use crate::gol::GameOfLife;

use calloop::timer::{TimeoutAction, Timer};
use calloop_wayland_source::WaylandSource;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop,
    globals::{BindError, GlobalList, GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat,
        wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use xkbcommon::xkb;

const CELL_SIZE: usize = 12;
const BUFFER_COUNT: usize = 3; // 2 may be enough
const TICK_MILLIS_MIN: u64 = 100;
const TICK_MILLIS_DEFAULT: u64 = 400;
const TICK_MILLIS_MAX: u64 = 1000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the Wayland server UNIX socket
    let conn = Connection::connect_to_env()?;

    // Get the globals and event queue using the registry
    let (globals, event_queue) = registry_queue_init::<AppState>(&conn)?;
    let qh = event_queue.handle();

    // Setup the global app state
    let mut app = AppState::new(&globals, &qh)?;

    // Setup the event loop
    let mut event_loop = calloop::EventLoop::<AppState>::try_new()?;

    // Insert the event_queue into the loop to handle server events
    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;

    // Setup timer source for updating the UI periodically
    let timer = Timer::from_duration(Duration::from_millis(app.tick_millis));
    event_loop.handle().insert_source(timer, |_, _, app| {
        // On every tick, advance the game one step and trigger an update
        if !app.paused {
            app.game.step();
            app.needs_update = true;
        }

        // Reset the timer
        TimeoutAction::ToDuration(Duration::from_millis(app.tick_millis))
    })?;

    // Start the event loop with the two sources
    let signal = event_loop.get_signal();
    event_loop.run(None, &mut app, |app| {
        // Compositor sent Close event (handled at Dispatch<XdgToplevel>)
        if app.quit {
            signal.stop();
        }
        // Timer triggered and update, so request a new frame
        if app.needs_update && app.configured && !app.frame_pending {
            app.request_frame(&qh);
        }
    })?;

    Ok(())
}

#[allow(dead_code)]
struct AppState {
    // Window state
    quit: bool,
    configured: bool,
    frame_pending: bool,
    needs_update: bool,
    width: usize,
    height: usize,

    // Game state
    game: GameOfLife,
    paused: bool,
    tick_millis: u64,

    // Dynamic globals
    wl_shm: wl_shm::WlShm, // useful to create new shm_pools if needed
    wl_surface: wl_surface::WlSurface,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    wl_pointer: Option<wl_pointer::WlPointer>,

    // Buffer pool
    mmap: memmap2::MmapMut,
    pool: wl_shm_pool::WlShmPool, // useful to create or resize buffers if needed
    buffers: Vec<PoolBuffer>,
    stride: usize,
    buf_size: usize,

    // Keyboard handling
    xkb_context: xkb::Context,
    xkb_state: Option<xkb::State>,

    // Pointer handling
    pointer_pos: (f64, f64),
    pointer_frame: PointerFrame, // to accum events until Frame
}

struct PoolBuffer {
    wl_buffer: wl_buffer::WlBuffer,
    busy: bool,
}

#[derive(Default)]
struct PointerFrame {
    motion: Option<(f64, f64)>,
    button: Option<(u32, bool)>, // button code, pressed
}

impl AppState {
    fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self, BindError> {
        // Obtain compositor object and create a wl_surface
        let compositor: wl_compositor::WlCompositor = globals.bind(qh, 1..=6, ())?;
        let wl_surface = compositor.create_surface(qh, ());

        // Get an xdg_surface from the wl_surface
        let xdg_wm_base: xdg_wm_base::XdgWmBase = globals.bind(qh, 1..=6, ())?;
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface, qh, ());

        // Get a toplevel (regular window) from the xdg_surface
        let xdg_toplevel = xdg_surface.get_toplevel(qh, ());
        xdg_toplevel.set_app_id("game_of_life".to_string());
        xdg_toplevel.set_title("Conway's Game of Life".to_string());

        // Get the SHM object for creating buffers on render
        let wl_shm: wl_shm::WlShm = globals.bind(qh, 1..=2, ())?;

        // Submit all created objects
        wl_surface.commit();

        // Create buffer pool
        let (width, height) = (600, 600);
        let stride = width * 4; // size of each pixel: XRGB = 4 bytes
        let buf_size = stride * height;
        let total_size = buf_size * BUFFER_COUNT;

        let file = tempfile::tempfile().unwrap();
        file.set_len(total_size as u64).unwrap();
        let mmap = unsafe { memmap2::MmapMut::map_mut(&file).unwrap() };
        let pool = wl_shm.create_pool(file.as_fd(), total_size as i32, qh, ());

        // Create buffers from pool
        let buffers = (0..BUFFER_COUNT)
            .map(|i| {
                let wl_buffer = pool.create_buffer(
                    (i * buf_size) as i32,
                    width as i32,
                    height as i32,
                    stride as i32,
                    wl_shm::Format::Xrgb8888,
                    qh,
                    i, // data: buffer index on pool for tracking
                );
                PoolBuffer { wl_buffer, busy: false }
            })
            .collect();

        // Get the seat global
        let _wl_seat: wl_seat::WlSeat = globals.bind(qh, 1..=9, ())?;

        // Setup GoL state
        let (width, height) = (600, 600);
        let mut game = GameOfLife::new(width / CELL_SIZE, height / CELL_SIZE);

        // Initial grid state (glider pattern)
        game.set_alive(0, 30);
        game.set_alive(1, 31);
        game.set_alive(2, 31);
        game.set_alive(0, 32);
        game.set_alive(1, 32);

        game.set_alive(0, 40);
        game.set_alive(1, 41);
        game.set_alive(2, 41);
        game.set_alive(0, 42);
        game.set_alive(1, 42);

        Ok(AppState {
            quit: false,
            configured: false,
            frame_pending: false,
            needs_update: false,
            width,
            height,
            game,
            paused: false,
            tick_millis: TICK_MILLIS_DEFAULT,
            wl_shm,
            wl_surface,
            wl_keyboard: None,
            wl_pointer: None,
            mmap,
            pool,
            buffers,
            stride,
            buf_size,
            xkb_context: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            xkb_state: None,
            pointer_pos: (0., 0.),
            pointer_frame: PointerFrame::default(),
        })
    }

    // Render the grid in an available buffer
    fn render_grid(&mut self) {
        let Some(idx) = self.buffers.iter().position(|b| !b.busy) else {
            // No buffer released by the compositor, skip frame
            return;
        };

        let (width, height) = (self.width, self.height);
        let grid_w = self.game.get_width();
        let stride = self.stride;

        let offset = idx * self.buf_size;
        let buffer = &mut self.mmap[offset..offset + self.buf_size];

        // Little-endian so B,G,R,X (no alpha)
        const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
        const BLACK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];

        for y in 0..height {
            let cell_y = y / CELL_SIZE;
            let row = &mut buffer[y * stride..(y + 1) * stride];
            for cell_x in 0..grid_w {
                let color = match self.game.at(cell_x, cell_y).unwrap().is_alive() {
                    true => WHITE,
                    false => BLACK,
                };
                for i in 0..CELL_SIZE {
                    let x0 = (cell_x * CELL_SIZE + i) * 4;
                    row[x0..x0 + 4].copy_from_slice(&color);
                }
            }
        }

        // Attach the buffer and trigger a re-render
        self.buffers[idx].busy = true;
        self.wl_surface.attach(Some(&self.buffers[idx].wl_buffer), 0, 0);
        self.wl_surface.damage_buffer(0, 0, width as i32, height as i32);
        self.wl_surface.commit();
    }

    /// Request a new frame to the wl_surface. The compositor will send a wl_callback.
    fn request_frame(&mut self, qh: &QueueHandle<Self>) {
        if self.frame_pending {
            return;
        }

        self.wl_surface.frame(qh, ());
        self.wl_surface.commit();
        self.frame_pending = true;
    }

    /// Handle a key pressed to perform an action
    fn handle_key(&mut self, key: xkb::Keysym) {
        use xkb::Keysym;
        match key {
            Keysym::space => self.paused = !self.paused,
            Keysym::plus | Keysym::Up => {
                self.tick_millis = TICK_MILLIS_MIN.max(self.tick_millis - 100)
            }
            Keysym::minus | Keysym::Down => {
                self.tick_millis = TICK_MILLIS_MAX.min(self.tick_millis + 100)
            }
            Keysym::r => {
                let cell_x = self.pointer_pos.0 as usize / CELL_SIZE;
                let cell_y = self.pointer_pos.1 as usize / CELL_SIZE;
                self.game.spawn_random_glider(cell_x, cell_y);
            }
            Keysym::c => self.game.clear(),
            Keysym::q => self.quit = true,
            _ => return,
        }
        self.needs_update = true;
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppState {
    fn event(
        _state: &mut AppState,
        _registry: &wl_registry::WlRegistry,
        _event: <wl_registry::WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<AppState>,
    ) {
        // TODO: Deal with dynamic globals e.g. plugged/unplugged monitors, input, etc.
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for AppState {
    fn event(
        _state: &mut AppState,
        proxy: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<AppState>,
    ) {
        // Ping handling
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for AppState {
    fn event(
        state: &mut AppState,
        proxy: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // Handle the end of XdgToplevel Configure sequences, rendering the game
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.configured = true;
            state.render_grid();
        };
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for AppState {
    fn event(
        state: &mut AppState,
        _proxy: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // TODO: Handle Configure events (resizes, fullscreen, etc)
        // Note: do NOT render here yet, only apply changes to internal state
        // Re-rendering should be performed at XdgSurface Configure event
        if let xdg_toplevel::Event::Close = event {
            state.quit = true;
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for AppState {
    fn event(
        state: &mut AppState,
        _proxy: &wl_callback::WlCallback,
        event: <wl_callback::WlCallback as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // The compositor signalled it's ok to send a new frame (response to wl_surface.frame)
        // so we render a new one
        if let wl_callback::Event::Done { .. } = event {
            state.frame_pending = false;
            state.render_grid();
            state.needs_update = false;
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, usize> for AppState {
    fn event(
        state: &mut AppState,
        _proxy: &wl_buffer::WlBuffer,
        event: <wl_buffer::WlBuffer as Proxy>::Event,
        data: &usize,
        _conn: &Connection,
        _qh: &QueueHandle<AppState>,
    ) {
        // Listen when the compositor releases a buffer from the pool
        if let wl_buffer::Event::Release = event {
            state.buffers[*data].busy = false;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for AppState {
    fn event(
        state: &mut AppState,
        seat: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<AppState>,
    ) {
        // Listen to seat capabilities to know if keyboard and pointer are supported
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let WEnum::Value(caps) = capabilities else { return };
            if caps.contains(wl_seat::Capability::Keyboard) && state.wl_keyboard.is_none() {
                state.wl_keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && state.wl_pointer.is_none() {
                state.wl_pointer = Some(seat.get_pointer(qh, ()));
            }
            // TODO: handle keyboard and pointer disconnections
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // TODO: handle repeat events
        // Handle key presses
        use wl_keyboard::Event;
        match event {
            Event::Keymap { format, fd, size } => {
                if format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    let keymap = unsafe {
                        xkb::Keymap::new_from_fd(
                            &state.xkb_context,
                            fd,
                            size as usize,
                            xkb::KEYMAP_FORMAT_TEXT_V1,
                            xkb::COMPILE_NO_FLAGS,
                        )
                    }
                    .ok()
                    .flatten();
                    state.xkb_state = keymap.map(|km| xkb::State::new(&km));
                }
            }
            Event::Key { key, state: key_state, .. } => {
                let Some(xkb_state) = &state.xkb_state else { return };
                let keysym = xkb_state.key_get_one_sym((key + 8).into());

                // Check is its a key press
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.handle_key(keysym);
                }
            }
            Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                if let Some(xkb_state) = &mut state.xkb_state {
                    xkb_state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Handle mouse events
        match event {
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.pointer_frame.motion = Some((surface_x, surface_y));
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                let pressed = btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                state.pointer_frame.button = Some((button, pressed));
            }
            wl_pointer::Event::Frame => {
                let frame = std::mem::take(&mut state.pointer_frame);
                if let Some(xy) = frame.motion {
                    state.pointer_pos = xy;
                }
                if let Some((button, pressed)) = frame.button
                    && pressed
                {
                    // From Linux kernel codes at input-event-codes.h
                    const BTN_LEFT: u32 = 0x110;
                    const BTN_RIGHT: u32 = 0x111;

                    let cell_x = state.pointer_pos.0 as usize / CELL_SIZE;
                    let cell_y = state.pointer_pos.1 as usize / CELL_SIZE;

                    // Left button: set cell alive
                    if button == BTN_LEFT {
                        state.game.set_alive(cell_x, cell_y);
                    } else if button == BTN_RIGHT {
                        state.game.set_dead(cell_x, cell_y);
                    }

                    state.needs_update = true;
                }
            }
            _ => {}
        }
    }
}

// These either don't emit events or we're not interested in handling them
delegate_noop!(AppState: ignore wl_compositor::WlCompositor);
delegate_noop!(AppState: ignore wl_surface::WlSurface);
delegate_noop!(AppState: ignore wl_shm::WlShm);
delegate_noop!(AppState: ignore wl_shm_pool::WlShmPool);
