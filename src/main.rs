use std::{os::fd::AsFd, time::Duration};

use calloop::timer::{TimeoutAction, Timer};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{BindError, GlobalList, GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

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
    let wl_source = calloop_wayland_source::WaylandSource::new(conn, event_queue);
    wl_source.insert(event_loop.handle())?;

    // Setup timer source for updating the UI periodically
    let timer = Timer::from_duration(Duration::from_millis(TICK_MILLIS));
    event_loop.handle().insert_source(timer, |_, _, app| {
        // On every tick, advance the game one step and trigger an update
        app.game_step();
        app.needs_update = true;

        // Reset the timer
        TimeoutAction::ToDuration(Duration::from_millis(TICK_MILLIS))
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

const CELL_SIZE: usize = 12;
const TICK_MILLIS: u64 = 1000;

#[derive(Debug)]
struct AppState {
    // Window state
    quit: bool,
    configured: bool,
    frame_pending: bool,
    needs_update: bool,
    width: usize,
    height: usize,

    // Game state
    grid: Vec<bool>,

    // Dynamic globals
    wl_shm: wl_shm::WlShm,
    wl_surface: wl_surface::WlSurface,
}

impl AppState {
    fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self, BindError> {
        // Obtain compositor object and create a wl_surface
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ())?;
        let wl_surface = compositor.create_surface(qh, ());

        // Get an xdg_surface from the wl_surface
        let xdg_wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ())?;
        let xdg_surface = xdg_wm_base.get_xdg_surface(&wl_surface, qh, ());

        // Get a toplevel (regular window) from the xdg_surface
        let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());
        xdg_toplevel.set_app_id("game_of_life".to_string());
        xdg_toplevel.set_title("Conway's Game of Life".to_string());

        // Get the SHM object for creating buffers on render
        let wl_shm: wl_shm::WlShm = globals.bind(qh, 1..=2, ())?;

        // Submit all created objects
        wl_surface.commit();

        // Setup rest of state
        let (width, height) = (600, 600);
        let mut grid = vec![false; (width / CELL_SIZE * height / CELL_SIZE) as usize];

        // Initial grid state (glider pattern)
        grid[0] = true;
        grid[51] = true;
        grid[52] = true;
        grid[100] = true;
        grid[101] = true;

        Ok(AppState {
            quit: false,
            configured: false,
            frame_pending: false,
            needs_update: false,
            width,
            height,
            grid,
            wl_shm,
            wl_surface,
        })
    }

    fn game_step(&mut self) {
        let grid_w = self.width as usize / CELL_SIZE;
        let grid_h = self.height as usize / CELL_SIZE;
        let mut next = self.grid.clone();

        for y in 0..grid_h {
            for x in 0..grid_w {
                let mut alive_neighbors = 0;

                for dy in [-1isize, 0, 1] {
                    for dx in [-1isize, 0, 1] {
                        let (i, j) = (x as isize + dx, y as isize + dy);
                        if (dx == 0 && dy == 0)
                            || (j < 0 || j >= grid_h as isize)
                            || (i < 0 || i >= grid_w as isize)
                        {
                            continue;
                        }
                        if self.grid[i as usize + j as usize * grid_w] {
                            alive_neighbors += 1;
                        }
                    }
                }

                next[x + y * grid_w] = match next[x + y * grid_w] {
                    true => (2..=3).contains(&alive_neighbors),
                    false => alive_neighbors == 3,
                }
            }
        }

        self.grid = next;
    }

    // FIX: This is extremely inefficient.
    // We're currently creating and destroying a temp file, a SHM pool and a buffer
    // on every re-render. It's ok since the app only renders 1 FPS but this should be
    // handled with double/triple mem-mapped or GPU buffers.
    fn render_grid(&mut self, qh: &QueueHandle<Self>) {
        use std::io::Write;
        let grid_w = self.width / CELL_SIZE;
        let grid_h = self.height / CELL_SIZE;
        let size = grid_w * grid_h * CELL_SIZE * 4;
        let (width, height) = (self.width as i32, self.height as i32);

        // Buffered writing to a temp file
        let mut buf_file = tempfile::tempfile().unwrap();
        let mut buf = std::io::BufWriter::with_capacity(size, &mut buf_file);

        for y in 0..self.width {
            for x in 0..self.height {
                let cell_x = x / CELL_SIZE;
                let cell_y = y / CELL_SIZE;
                let bytes = match self.grid[cell_x + cell_y * grid_w] {
                    // Little-endian so B,G,R,X (no alpha)
                    true => [0xFF, 0xFF, 0xFF, 0xFF],  // white
                    false => [0x00, 0x00, 0x00, 0xFF], // black
                };
                buf.write_all(&bytes).unwrap();
            }
        }

        drop(buf);

        // Create the SHM pool and a buffer from it
        let pool = self.wl_shm.create_pool(buf_file.as_fd(), width * height * 4, &qh, ());
        let wl_buffer =
            pool.create_buffer(0, width, height, width * 4, wl_shm::Format::Xrgb8888, &qh, ());

        // Attach the buffer and trigger a re-render
        self.wl_surface.attach(Some(&wl_buffer), 0, 0);
        self.wl_surface.damage_buffer(0, 0, width, height);
        self.wl_surface.commit();
        pool.destroy();
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
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppState {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: <wl_registry::WlRegistry as wayland_client::Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        // TODO: Deal with dynamic globals e.g. plugged/unplugged monitors, input, etc.
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for AppState {
    fn event(
        _state: &mut Self,
        proxy: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        // Ping handling
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // Handle the end of XdgToplevel Configure sequences, rendering the game
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.configured = true;
            state.render_grid(qh);
        };
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
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
        app: &mut Self,
        _callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // The compositor signalled it's ok to send a new frame (response to wl_surface.frame)
        // so we render a new one
        if let wl_callback::Event::Done { .. } = event {
            app.frame_pending = false;
            app.render_grid(qh);
            app.needs_update = false;
        }
    }
}

// These either don't emit events or we're not interested in handling them
delegate_noop!(AppState: ignore wl_compositor::WlCompositor);
delegate_noop!(AppState: ignore wl_surface::WlSurface);
delegate_noop!(AppState: ignore wl_shm::WlShm);
delegate_noop!(AppState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(AppState: ignore wl_buffer::WlBuffer);
