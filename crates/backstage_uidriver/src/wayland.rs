//! The Wayland side: a virtual pointer and keyboard, and output capture,
//! through wlroots' protocols (cage implements all three).

use crate::commands::{Key, bgra_to_rgba};
use anyhow::{Context, Result, bail};
use memmap2::MmapMut;
use std::io::Write;
use std::os::fd::AsFd;
use std::path::Path;
use std::time::Instant;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
/// `wl_keyboard.keymap_format.xkb_v1`.
const KEYMAP_XKB_V1: u32 = 1;
/// The standard evdev US layout, as includes the compositor resolves with
/// its own xkb data (what `setxkbmap -print` gives for it).
const KEYMAP: &str = r#"xkb_keymap {
    xkb_keycodes { include "evdev+aliases(qwerty)" };
    xkb_types { include "complete" };
    xkb_compat { include "complete" };
    xkb_symbols { include "pc+us+inet(evdev)" };
    xkb_geometry { include "pc(pc105)" };
};
"#;

#[derive(Default)]
struct State {
    /// The output's current mode, in pixels.
    output_size: Option<(i32, i32)>,
    capture: Capture,
}

/// Progress of one screencopy.
#[derive(Default)]
struct Capture {
    /// `(format, width, height, stride)` of a shm buffer the compositor
    /// can copy into.
    buffer: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    ready: bool,
    failed: bool,
}

pub struct Driver {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    output: wl_output::WlOutput,
    shm: wl_shm::WlShm,
    pointer: ZwlrVirtualPointerV1,
    keyboard: ZwpVirtualKeyboardV1,
    screencopy: ZwlrScreencopyManagerV1,
    start: Instant,
    /// Modifier bits currently held on the virtual keyboard.
    mods: u32,
}

impl Driver {
    /// Connects to the compositor in `WAYLAND_DISPLAY` and creates the
    /// virtual devices.
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland compositor")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).context("listing Wayland globals")?;
        let qh = queue.handle();
        let bind_err =
            |what: &str| format!("the compositor has no {what} (is this cage or another wlroots one?)");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).with_context(|| bind_err("wl_seat"))?;
        let output: wl_output::WlOutput =
            globals.bind(&qh, 1..=4, ()).with_context(|| bind_err("wl_output"))?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).with_context(|| bind_err("wl_shm"))?;
        let pointers: ZwlrVirtualPointerManagerV1 =
            globals.bind(&qh, 1..=2, ()).with_context(|| bind_err("virtual pointer manager"))?;
        let keyboards: ZwpVirtualKeyboardManagerV1 =
            globals.bind(&qh, 1..=1, ()).with_context(|| bind_err("virtual keyboard manager"))?;
        let screencopy: ZwlrScreencopyManagerV1 =
            globals.bind(&qh, 1..=3, ()).with_context(|| bind_err("screencopy manager"))?;

        let pointer = pointers.create_virtual_pointer(Some(&seat), &qh, ());
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());
        let keymap = shm_file(KEYMAP.len() + 1)?;
        (&keymap).write_all(KEYMAP.as_bytes())?;
        (&keymap).write_all(&[0])?;
        keyboard.keymap(KEYMAP_XKB_V1, keymap.as_fd(), KEYMAP.len() as u32 + 1);

        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        if state.output_size.is_none() {
            bail!("the output reported no mode");
        }
        Ok(Self {
            conn,
            queue,
            state,
            output,
            shm,
            pointer,
            keyboard,
            screencopy,
            start: Instant::now(),
            mods: 0,
        })
    }

    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    /// Waits until the compositor has handled everything sent so far.
    fn sync(&mut self) -> Result<()> {
        self.queue.roundtrip(&mut self.state).context("Wayland roundtrip")?;
        Ok(())
    }

    pub fn move_to(&mut self, x: f64, y: f64) -> Result<()> {
        let (w, h) = self.state.output_size.unwrap_or((1, 1));
        let x = x.round().clamp(0.0, (w - 1) as f64) as u32;
        let y = y.round().clamp(0.0, (h - 1) as f64) as u32;
        self.pointer.motion_absolute(self.time(), x, y, w as u32, h as u32);
        self.pointer.frame();
        self.sync()
    }

    pub fn button(&mut self, pressed: bool) -> Result<()> {
        let state =
            if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
        self.pointer.button(self.time(), BTN_LEFT, state);
        self.pointer.frame();
        self.sync()
    }

    pub fn key(&mut self, key: Key, pressed: bool) -> Result<()> {
        // wl_keyboard.key_state: 1 pressed, 0 released.
        self.keyboard.key(self.time(), key.evdev(), pressed as u32);
        let mask = key.modifier_mask();
        if mask != 0 {
            self.mods = if pressed { self.mods | mask } else { self.mods & !mask };
            self.keyboard.modifiers(self.mods, 0, 0, 0);
        }
        self.sync()
    }

    /// Captures the whole output into a PNG at `path`.
    pub fn screenshot(&mut self, path: &Path) -> Result<()> {
        let qh = self.queue.handle();
        self.state.capture = Capture::default();
        let frame = self.screencopy.capture_output(0, &self.output, &qh, ());
        // Version 3 lists every buffer type, then says done; older ones
        // offer the shm buffer alone.
        let needs_done = frame.version() >= 3;
        while !self.state.capture.failed
            && !(self.state.capture.buffer.is_some() && (self.state.capture.buffer_done || !needs_done))
        {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let Some((format, width, height, stride)) =
            self.state.capture.buffer.filter(|_| !self.state.capture.failed)
        else {
            frame.destroy();
            bail!("the compositor refused the screen capture");
        };
        let size = stride as usize * height as usize;
        let file = shm_file(size)?;
        let pool = self.shm.create_pool(file.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, format, &qh, ());
        frame.copy(&buffer);
        while !self.state.capture.ready && !self.state.capture.failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let failed = self.state.capture.failed;
        frame.destroy();
        buffer.destroy();
        pool.destroy();
        self.conn.flush()?;
        if failed {
            bail!("the screen capture failed");
        }
        // SAFETY: the file is ours alone (unlinked), and the compositor has
        // finished writing it (`ready`).
        let map = unsafe { MmapMut::map_mut(&file)? };
        let opaque = format == wl_shm::Format::Xrgb8888;
        let rgba = bgra_to_rgba(&map, width, height, stride, opaque);
        image::save_buffer(path, &rgba, width, height, image::ColorType::Rgba8)
            .with_context(|| format!("writing {}", path.display()))
    }
}

/// An anonymous file of `size` bytes to share with the compositor.
fn shm_file(size: usize) -> Result<std::fs::File> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let path = Path::new(&dir).join(format!(
        "backstage-uidriver-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path)?;
    std::fs::remove_file(&path)?;
    file.set_len(size as u64)?;
    Ok(file)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } = event
            && flags.contains(wl_output::Mode::Current)
        {
            state.output_size = Some((width, height));
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_screencopy_frame_v1::Event;
        let capture = &mut state.capture;
        match event {
            Event::Buffer { format: WEnum::Value(format), width, height, stride }
                if matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888) =>
            {
                capture.buffer = Some((format, width, height, stride));
            }
            Event::BufferDone => capture.buffer_done = true,
            Event::Ready { .. } => capture.ready = true,
            Event::Failed => capture.failed = true,
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);
delegate_noop!(State: ZwlrScreencopyManagerV1);
