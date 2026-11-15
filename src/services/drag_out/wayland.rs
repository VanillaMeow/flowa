//! A Wayland drag source on the window's own connection
//!
//! A second event queue binds its own seat, pointer, and data device. Pointer events
//! reach every pointer object of a client, so this queue learns the serial of the
//! button press that a drag must name. A thread dispatches the queue for the life of
//! the window, answering data requests while the compositor runs the drag

use std::{
    ffi::c_void,
    fs::File,
    io::Write,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use anyhow::{Context, Result};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_offer::WlDataOffer,
        wl_data_source::{self, WlDataSource},
        wl_pointer::{self, WlPointer},
        wl_registry::WlRegistry,
        wl_seat::{self, WlSeat},
        wl_surface::WlSurface,
    },
};

use super::{DragSource, URI_LIST};

pub(super) struct WaylandSource {
    connection: Connection,
    queue: QueueHandle<State>,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    surface: WlSurface,
    events: Arc<Events>,
}

/// What the dispatch thread tells the UI thread
struct Events {
    /// The latest button press on any of this client's surfaces, or zero
    press_serial: AtomicU32,
    finished: Arc<AtomicBool>,
    ctx: egui::Context,
}

struct State {
    events: Arc<Events>,
    pointer: Option<WlPointer>,
    /// Offers this data device received, released when replaced
    selection: Option<WlDataOffer>,
    entered: Option<WlDataOffer>,
}

impl WaylandSource {
    pub fn new(
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
        finished: Arc<AtomicBool>,
        ctx: egui::Context,
    ) -> Result<Self> {
        // SAFETY: Winit keeps the display alive for the life of the window, which owns
        // this source
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
        let handle = queue.handle();

        let seat: WlSeat = globals.bind(&handle, 1..=7, ())?;
        let manager: WlDataDeviceManager = globals.bind(&handle, 1..=3, ())?;
        let device = manager.get_data_device(&seat, &handle, ());

        // SAFETY: The pointer is winit's live surface, which outlives this source
        let surface_id = unsafe {
            ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast())
        }
        .context("The window surface is not a wl_surface")?;
        let surface = WlSurface::from_id(&connection, surface_id)?;

        let events = Arc::new(Events {
            press_serial: AtomicU32::new(0),
            finished,
            ctx,
        });
        let mut state = State {
            events: Arc::clone(&events),
            pointer: None,
            selection: None,
            entered: None,
        };

        queue.roundtrip(&mut state)?;
        crate::services::worker::spawn("wayland-drag", move || {
            while queue.blocking_dispatch(&mut state).is_ok() {}
        });

        Ok(Self {
            connection,
            queue: handle,
            manager,
            device,
            surface,
            events,
        })
    }
}

impl DragSource for WaylandSource {
    fn start(&mut self, uris: Vec<u8>) -> Result<()> {
        let serial = self.events.press_serial.load(Ordering::SeqCst);
        anyhow::ensure!(serial != 0, "No pointer press started this drag");

        let source = self.manager.create_data_source(&self.queue, Arc::new(uris));
        source.offer(URI_LIST.into());

        if source.version() >= 3 {
            source.set_actions(DndAction::Copy);
        }

        self.device
            .start_drag(Some(&source), &self.surface, None, serial);
        self.connection.flush()?;

        Ok(())
    }
}

impl Events {
    fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
        self.ctx.request_repaint();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };

        if capabilities.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(handle, ()));
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            state.events.press_serial.store(serial, Ordering::SeqCst);
        }
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Winit handles drops and the clipboard; this device only releases its offers
        match event {
            wl_data_device::Event::Selection { id } => {
                release(&mut state.selection, id)
            }
            wl_data_device::Event::Enter { id, .. } => release(&mut state.entered, id),
            wl_data_device::Event::Leave | wl_data_device::Event::Drop => {
                release(&mut state.entered, None);
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        _: <WlDataOffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataSource, Arc<Vec<u8>>> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        uris: &Arc<Vec<u8>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } if mime_type == URI_LIST => {
                if let Err(error) = File::from(fd).write_all(uris) {
                    log::warn!("Cannot send dragged files: {error}");
                }
            }
            wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => {
                source.destroy();
                state.events.finish();
            }
            wl_data_source::Event::DndDropPerformed => state.events.finish(),
            _ => {}
        }
    }
}

/// Keeps the newest offer and destroys the one it replaces
fn release(slot: &mut Option<WlDataOffer>, replacement: Option<WlDataOffer>) {
    if let Some(previous) = std::mem::replace(slot, replacement) {
        previous.destroy();
    }
}
