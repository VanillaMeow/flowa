//! An X11 drag source speaking the XDND protocol on its own connection
//!
//! Pressing the button gives winit's connection an automatic pointer grab, which only
//! that connection can release. The UI thread releases it through Xlib, then a thread
//! with its own connection grabs the pointer, finds XDND-aware windows under it, and
//! serves the file list as the `XdndSelection`

use std::{
    ffi::{c_ulong, c_void},
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use x11rb::{
    CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{
        Event,
        xproto::{
            Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux,
            EventMask, GrabMode, GrabStatus, PropMode, SelectionNotifyEvent,
            SelectionRequestEvent, Timestamp, Window, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use super::{DragSource, URI_LIST};

/// The newest XDND version this source speaks
const XDND_VERSION: u32 = 5;
/// How long to wait for the target to read the files after the drop
const DROP_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the server may take to release winit's grab before ours succeeds
const GRAB_ATTEMPTS: u32 = 40;

pub(super) struct X11Source {
    xlib: x11_dl::xlib::Xlib,
    display: NonNull<c_void>,
    drags: mpsc::Sender<Vec<u8>>,
}

struct Atoms {
    aware: Atom,
    selection: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    copy: Atom,
    targets: Atom,
    uri_list: Atom,
}

/// The connection, its hidden selection window, and the window drags must skip
struct Session {
    connection: RustConnection,
    root: Window,
    source: Window,
    own_window: Window,
    atoms: Atoms,
}

/// The XDND-aware window under the pointer and whether it accepts the files
#[derive(Clone, Copy)]
struct Target {
    window: Window,
    version: u32,
    accepted: bool,
}

impl X11Source {
    pub fn new(
        display: Option<NonNull<c_void>>,
        window: c_ulong,
        finished: Arc<AtomicBool>,
        ctx: egui::Context,
    ) -> Result<Self> {
        let display = display.context("Winit did not report its X display")?;
        let xlib = x11_dl::xlib::Xlib::open()?;
        let session = Session::connect(None, window as Window)?;
        let (drags, requests) = mpsc::channel::<Vec<u8>>();

        crate::services::worker::spawn("x11-drag", move || {
            while let Ok(uris) = requests.recv() {
                if let Err(error) = session.drag(&uris) {
                    log::warn!("The drag ended early: {error:#}");
                }

                finished.store(true, Ordering::SeqCst);
                ctx.request_repaint();
            }
        });

        Ok(Self {
            xlib,
            display,
            drags,
        })
    }
}

impl DragSource for X11Source {
    fn start(&mut self, uris: Vec<u8>) -> Result<()> {
        // SAFETY: Winit's display outlives the window that owns this source, and winit
        // initialized Xlib for threads
        unsafe {
            let display = self.display.as_ptr().cast();
            (self.xlib.XUngrabPointer)(display, 0);
            (self.xlib.XFlush)(display);
        }

        self.drags
            .send(uris)
            .map_err(|_| anyhow::anyhow!("The drag thread stopped"))
    }
}

impl Session {
    /// Connects to `display`, or to `DISPLAY` when none is given
    fn connect(display: Option<&str>, own_window: Window) -> Result<Self> {
        let (connection, screen) = x11rb::connect(display)?;
        let root = connection.setup().roots[screen].root;
        let source = connection.generate_id()?;

        connection.create_window(
            0,
            source,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().override_redirect(1),
        )?;

        let atoms = Atoms::intern(&connection)?;
        connection.flush()?;

        Ok(Self {
            connection,
            root,
            source,
            own_window,
            atoms,
        })
    }

    /// Runs one drag until the button is released and the target has the files
    fn drag(&self, uris: &[u8]) -> Result<()> {
        let connection = &self.connection;
        connection.set_selection_owner(
            self.source,
            self.atoms.selection,
            CURRENT_TIME,
        )?;
        self.grab()?;

        let result = self.follow_pointer(uris);

        connection.ungrab_pointer(CURRENT_TIME)?;
        connection.flush()?;
        result
    }

    fn grab(&self) -> Result<()> {
        let mask = EventMask::POINTER_MOTION | EventMask::BUTTON_RELEASE;

        for _ in 0..GRAB_ATTEMPTS {
            let status = self
                .connection
                .grab_pointer(
                    false,
                    self.root,
                    mask,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    NONE,
                    NONE,
                    CURRENT_TIME,
                )?
                .reply()?
                .status;

            if status == GrabStatus::SUCCESS {
                return Ok(());
            }

            thread::sleep(Duration::from_millis(5));
        }

        anyhow::bail!("Cannot take over the pointer from the window")
    }

    fn follow_pointer(&self, uris: &[u8]) -> Result<()> {
        let mut target: Option<Target> = None;

        loop {
            match self.connection.wait_for_event()? {
                Event::MotionNotify(motion) => {
                    target = self.retarget(target)?;

                    if let Some(target) = target {
                        let point = (u32::from(motion.root_x as u16) << 16)
                            | u32::from(motion.root_y as u16);
                        let data =
                            [self.source, 0, point, motion.time, self.atoms.copy];
                        self.send(target.window, self.atoms.position, data)?;
                    }
                }
                Event::ClientMessage(message) if message.type_ == self.atoms.status => {
                    let data = message.data.as_data32();

                    if let Some(target) =
                        target.as_mut().filter(|t| t.window == data[0])
                    {
                        target.accepted = data[1] & 1 == 1;
                    }
                }
                Event::SelectionRequest(request) => self.answer(&request, uris)?,
                Event::ButtonRelease(release) => {
                    return self.release(target, release.time, uris);
                }
                _ => {}
            }
        }
    }

    /// Leaves the previous target and enters the window now under the pointer
    fn retarget(&self, current: Option<Target>) -> Result<Option<Target>> {
        let under = self.aware_window_under_pointer()?;

        if current.map(|target| target.window) == under.map(|(window, _)| window) {
            return Ok(current);
        }

        if let Some(previous) = current {
            self.send(previous.window, self.atoms.leave, [self.source, 0, 0, 0, 0])?;
        }

        let Some((window, version)) = under else {
            return Ok(None);
        };

        let version = version.min(XDND_VERSION);
        let data = [self.source, version << 24, self.atoms.uri_list, 0, 0];
        self.send(window, self.atoms.enter, data)?;

        Ok(Some(Target {
            window,
            version,
            accepted: false,
        }))
    }

    /// Drops onto an accepting target and waits for it to read the files
    fn release(
        &self,
        target: Option<Target>,
        time: Timestamp,
        uris: &[u8],
    ) -> Result<()> {
        let Some(target) = target else {
            return Ok(());
        };

        if !target.accepted {
            return self.send(
                target.window,
                self.atoms.leave,
                [self.source, 0, 0, 0, 0],
            );
        }

        self.send(target.window, self.atoms.drop, [self.source, 0, time, 0, 0])?;

        // Targets before version 2 never report finishing, so the selection is served
        // until the timeout
        let deadline = Instant::now() + DROP_TIMEOUT;

        while Instant::now() < deadline {
            let Some(event) = self.connection.poll_for_event()? else {
                thread::sleep(Duration::from_millis(5));
                continue;
            };

            match event {
                Event::SelectionRequest(request) => self.answer(&request, uris)?,
                Event::ClientMessage(message)
                    if message.type_ == self.atoms.finished && target.version >= 2 =>
                {
                    return Ok(());
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// The innermost window under the pointer that accepts XDND, skipping flowa's own
    fn aware_window_under_pointer(&self) -> Result<Option<(Window, u32)>> {
        let mut window = self.root;

        loop {
            let child = self.connection.query_pointer(window)?.reply()?.child;

            if child == NONE || child == self.own_window {
                return Ok(None);
            }

            let version = self
                .connection
                .get_property(false, child, self.atoms.aware, AtomEnum::ATOM, 0, 1)?
                .reply()?
                .value32()
                .and_then(|mut values| values.next());

            if let Some(version) = version {
                return Ok(Some((child, version)));
            }

            window = child;
        }
    }

    /// Writes the file list, or the list of offered types, into the requestor's property
    fn answer(&self, request: &SelectionRequestEvent, uris: &[u8]) -> Result<()> {
        let connection = &self.connection;
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };

        let served = if request.target == self.atoms.uri_list {
            connection.change_property8(
                PropMode::REPLACE,
                request.requestor,
                property,
                self.atoms.uri_list,
                uris,
            )?;
            true
        } else if request.target == self.atoms.targets {
            let offered = [self.atoms.targets, self.atoms.uri_list];
            connection.change_property32(
                PropMode::REPLACE,
                request.requestor,
                property,
                AtomEnum::ATOM,
                &offered,
            )?;
            true
        } else {
            false
        };

        let notify = SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if served { property } else { NONE },
        };
        connection.send_event(false, request.requestor, EventMask::NO_EVENT, notify)?;
        connection.flush()?;

        Ok(())
    }

    fn send(&self, window: Window, kind: Atom, data: [u32; 5]) -> Result<()> {
        let message = ClientMessageEvent::new(32, window, kind, data);

        self.connection
            .send_event(false, window, EventMask::NO_EVENT, message)?;
        self.connection.flush()?;

        Ok(())
    }
}

impl Atoms {
    fn intern(connection: &RustConnection) -> Result<Self> {
        let atom = |name: &str| -> Result<Atom> {
            Ok(connection
                .intern_atom(false, name.as_bytes())?
                .reply()?
                .atom)
        };

        Ok(Self {
            aware: atom("XdndAware")?,
            selection: atom("XdndSelection")?,
            enter: atom("XdndEnter")?,
            position: atom("XdndPosition")?,
            status: atom("XdndStatus")?,
            leave: atom("XdndLeave")?,
            drop: atom("XdndDrop")?,
            finished: atom("XdndFinished")?,
            copy: atom("XdndActionCopy")?,
            targets: atom("TARGETS")?,
            uri_list: atom(URI_LIST)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{process::Command, sync::mpsc};

    use super::*;

    /// A window that accepts drops and reports the files it reads
    fn drop_target(display: &str, files: mpsc::Sender<Vec<u8>>) -> Result<()> {
        let (connection, screen) = x11rb::connect(Some(display))?;
        let root = connection.setup().roots[screen].root;
        let window = connection.generate_id()?;
        let atoms = Atoms::intern(&connection)?;
        let data_property = connection.intern_atom(false, b"FLOWA_TEST")?.reply()?.atom;

        connection.create_window(
            0,
            window,
            root,
            300,
            300,
            200,
            200,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        connection.change_property32(
            PropMode::REPLACE,
            window,
            atoms.aware,
            AtomEnum::ATOM,
            &[XDND_VERSION],
        )?;
        connection.map_window(window)?;
        connection.flush()?;

        thread::spawn(move || -> Result<()> {
            let mut source = NONE;

            loop {
                match connection.wait_for_event()? {
                    Event::ClientMessage(message) => {
                        let data = message.data.as_data32();
                        source = data[0];

                        if message.type_ == atoms.position {
                            let status = [window, 1, 0, 0, atoms.copy];
                            let reply = ClientMessageEvent::new(
                                32,
                                source,
                                atoms.status,
                                status,
                            );
                            connection.send_event(
                                false,
                                source,
                                EventMask::NO_EVENT,
                                reply,
                            )?;
                        } else if message.type_ == atoms.drop {
                            connection.convert_selection(
                                window,
                                atoms.selection,
                                atoms.uri_list,
                                data_property,
                                data[2],
                            )?;
                        }
                        connection.flush()?;
                    }
                    Event::SelectionNotify(notify) if notify.property != NONE => {
                        let value = connection
                            .get_property(
                                true,
                                window,
                                data_property,
                                AtomEnum::ANY,
                                0,
                                4096,
                            )?
                            .reply()?
                            .value;
                        let _ = files.send(value);

                        let finished = [window, 1, atoms.copy, 0, 0];
                        let message = ClientMessageEvent::new(
                            32,
                            source,
                            atoms.finished,
                            finished,
                        );
                        connection.send_event(
                            false,
                            source,
                            EventMask::NO_EVENT,
                            message,
                        )?;
                        connection.flush()?;
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    fn xdotool(display: &str, arguments: &[&str]) -> Result<()> {
        let status = Command::new("xdotool")
            .args(arguments)
            .env("DISPLAY", display)
            .status()?;
        anyhow::ensure!(status.success(), "xdotool {arguments:?} failed");

        Ok(())
    }

    #[test]
    #[ignore = "Requires Xvfb and xdotool"]
    fn a_drag_delivers_the_file_list_to_an_xdnd_target() -> Result<()> {
        let display = ":91";
        let mut server = Command::new("Xvfb")
            .args([display, "-screen", "0", "800x600x24", "-nolisten", "tcp"])
            .spawn()?;
        let result = (|| -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(5);
            while x11rb::connect(Some(display)).is_err() {
                anyhow::ensure!(Instant::now() < deadline, "Xvfb did not start");
                thread::sleep(Duration::from_millis(50));
            }

            let (files, received) = mpsc::channel();
            drop_target(display, files)?;

            let session = Session::connect(Some(display), NONE)?;
            xdotool(display, &["mousemove", "20", "20", "mousedown", "1"])?;
            let dragging =
                thread::spawn(move || session.drag(b"file:///tmp/a%20b.png\r\n"));

            thread::sleep(Duration::from_millis(200));
            for (x, y) in [("100", "100"), ("320", "320"), ("380", "380")] {
                xdotool(display, &["mousemove", x, y])?;
                thread::sleep(Duration::from_millis(50));
            }
            xdotool(display, &["mouseup", "1"])?;

            let delivered = received.recv_timeout(Duration::from_secs(5))?;
            anyhow::ensure!(
                delivered == b"file:///tmp/a%20b.png\r\n",
                "The target read {:?}",
                String::from_utf8_lossy(&delivered)
            );

            dragging
                .join()
                .map_err(|_| anyhow::anyhow!("The drag panicked"))?
        })();

        let _ = server.kill();
        result
    }
}
