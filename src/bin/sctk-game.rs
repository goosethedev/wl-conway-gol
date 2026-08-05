use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shell::xdg;
use smithay_client_toolkit::{delegate_compositor, delegate_registry, registry, registry_handlers};
use wayland_client::{
    Connection, QueueHandle,
    globals::{BindError, GlobalList, registry_queue_init},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the Wayland server UNIX socket
    let conn = Connection::connect_to_env()?;

    // Get the globals and event queue using the registry
    let (globals, event_queue) = registry_queue_init::<AppState>(&conn)?;
    let qh = event_queue.handle();

    // Setup the global app state
    let mut app = AppState::new(&globals, &qh);

    // Setup the event loop
    let mut event_loop = calloop::EventLoop::<AppState>::try_new()?;

    // Register the queue into the loop
    WaylandSource::new(conn, event_queue).insert(event_loop.handle())?;

    Ok(())
}

struct AppState {
    registry_state: registry::RegistryState,
}

impl AppState {
    fn new(globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self, BindError> {
        let compositor = compositor::CompositorState::bind(globals, qh)?;
        let xdg_shell = xdg::XdgShell::bind(globals, qh)?;
        let surface = compositor.create_surface(qh);
        let window =
            xdg_shell.create_window(surface, xdg::window::WindowDecorations::RequestServer, qh);
        Ok(Self { registry_state: registry::RegistryState::new(globals) })
    }

    fn render_grid(&self) {}
}

delegate_registry!(AppState);
delegate_compositor!(AppState);

impl registry::ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut registry::RegistryState {
        &mut self.registry_state
    }

    registry_handlers!([OutputState]);
}

impl compositor::CompositorHandler for AppState {
    fn frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        time: u32,
    ) {
        self.render_grid();
    }

    fn surface_enter(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        output: &wayland_client::protocol::wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        output: &wayland_client::protocol::wl_output::WlOutput,
    ) {
    }
    fn transform_changed(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }
}
