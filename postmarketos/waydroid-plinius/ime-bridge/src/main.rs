// SPDX-License-Identifier: GPL-3.0-or-later
//! Bridges the Phosh on-screen keyboard to Waydroid.
//!
//! Waydroid does not implement text-input, and it forwards raw key codes to
//! Android without the keymap, so neither the on-screen keyboard nor its
//! virtual-keyboard fallback can type into Android apps. This daemon works
//! around both:
//!
//! * The "pmOS keyboard" Android input method connects to us over TCP on the
//!   Waydroid bridge and sends `SHOW <hint> <purpose>` when an Android text
//!   field wants a keyboard and `HIDE` when it no longer does.
//! * On `SHOW` we map a 1x1 transparent overlay layer surface that takes the
//!   keyboard focus and enables text-input-v3 on it, so Phosh raises the
//!   on-screen keyboard as for any native app.
//! * Text-input events (commit, preedit, delete) and key presses on that
//!   surface are translated using the keymap and sent back to the input
//!   method, which applies them to the focused Android field.
//!
//! Protocol: UTF-8 lines; `\\`, `\n` and `\r` inside arguments are escaped.
//! Android -> host: `HELLO <token>`, `SHOW <hint> <purpose>`, `HIDE`.
//! Host -> Android: `COMMIT <text>`, `PREEDIT <text>`,
//! `DELETE <before bytes> <after bytes>`, `KEY <android keycode> <meta>`.
//! The first token seen is remembered and other clients are refused, so other
//! Android apps cannot connect and read what is typed.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use wayland_client::backend::{ObjectId, WaylandError};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_region, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::text_input::zv3::client::{zwp_text_input_manager_v3, zwp_text_input_v3};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use xkbcommon::xkb;

const DEFAULT_ADDR: &str = "192.168.240.1:48613";
/// Android hides and shows the input method when focus moves between
/// fields; wait a little before dropping the keyboard to avoid flicker.
const HIDE_DELAY: Duration = Duration::from_millis(300);
const BIND_RETRY: Duration = Duration::from_secs(2);
const MAX_LINE: usize = 64 * 1024;
const REGRAB_WINDOW: Duration = Duration::from_secs(2);
const MAX_REGRABS: usize = 3;
const ORPHAN_DELAY: Duration = Duration::from_secs(1);

// Android KeyEvent constants
const META_SHIFT: u32 = 0x41;
const META_CTRL: u32 = 0x3000;
const KEYCODE_A: u32 = 29;

#[derive(Default)]
struct Toplevel {
    app_id: String,
    activated: bool,
}

struct App {
    compositor: wl_compositor::WlCompositor,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    text_input: zwp_text_input_v3::ZwpTextInputV3,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    buffer: wl_buffer::WlBuffer,
    surface: Option<wl_surface::WlSurface>,
    layer: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    entered: bool,
    enabled: bool,
    content: (u32, u32),
    pending_preedit: Option<String>,
    pending_commit: Option<String>,
    pending_delete: Option<(u32, u32)>,
    preedit: String,
    xkb_ctx: xkb::Context,
    xkb_state: Option<xkb::State>,
    track_toplevels: bool,
    toplevels: HashMap<ObjectId, Toplevel>,
    out: Vec<String>,
    hide_at: Option<Instant>,
    /// Set when the last Waydroid window closed; Android relaunches an
    /// activity in a new window when Phosh resizes it for the keyboard.
    orphan_at: Option<Instant>,
    regrabs: Vec<Instant>,
    /// Width and tallest height of the usable area seen at that width.
    full: (i32, i32),
    /// Keyboard height last sent to Android.
    inset: i32,
}

fn escape(s: &str) -> String {
    let mut r = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => r.push_str("\\\\"),
            '\n' => r.push_str("\\n"),
            '\r' => r.push_str("\\r"),
            c => r.push(c),
        }
    }
    r
}

fn is_waydroid(app_id: &str) -> bool {
    app_id.to_ascii_lowercase().starts_with("waydroid")
}

fn debug(args: std::fmt::Arguments) {
    if std::env::var_os("WAYDROID_IME_DEBUG").is_some() {
        eprintln!("waydroid-ime-bridge: {args}");
    }
}

impl App {
    /// Whether a Waydroid window is in front. Without the foreign toplevel
    /// protocol we cannot tell and assume it is.
    fn waydroid_active(&self) -> bool {
        if !self.track_toplevels {
            return true;
        }
        self.toplevels.values().any(|t| t.activated && is_waydroid(&t.app_id))
    }

    fn show(&mut self, qh: &QueueHandle<App>, hint: u32, purpose: u32) {
        self.hide_at = None;
        self.content = (hint, purpose);
        if self.surface.is_none() && !self.waydroid_active() {
            // An Android field got focus while another app is in front.
            debug(format_args!("SHOW ignored, Waydroid is not in front"));
            return;
        }
        if self.surface.is_none() {
            let surface = self.compositor.create_surface(qh, ());
            // Never take touches; the surface only exists to hold the focus.
            let region = self.compositor.create_region(qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
            let layer = self.layer_shell.get_layer_surface(
                &surface,
                None,
                zwlr_layer_shell_v1::Layer::Top,
                "waydroid-ime-bridge".into(),
                qh,
                (),
            );
            // Spanning the usable area, which shrinks by the height of the
            // keyboard: Android is told about it as the input method inset
            // (the hwcomposer shim keeps the Android display size).
            layer.set_size(0, 0);
            layer.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top
                    | zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            );
            layer.set_keyboard_interactivity(
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
            );
            surface.commit();
            self.surface = Some(surface);
            self.layer = Some(layer);
        } else if self.entered {
            self.enable();
        }
    }

    fn enable(&mut self) {
        let (hint, purpose) = self.content;
        let ti = &self.text_input;
        ti.enable();
        ti.set_content_type(
            zwp_text_input_v3::ContentHint::from_bits_truncate(hint),
            zwp_text_input_v3::ContentPurpose::try_from(purpose)
                .unwrap_or(zwp_text_input_v3::ContentPurpose::Normal),
        );
        ti.set_cursor_rectangle(0, 0, 1, 1);
        ti.commit();
        self.enabled = true;
    }

    fn hide(&mut self) {
        debug(format_args!("hide (surface={})", self.surface.is_some()));
        if self.inset != 0 {
            self.inset = 0;
            self.out.push("INSET 0 0".into());
        }
        self.hide_at = None;
        self.orphan_at = None;
        if self.enabled {
            self.text_input.disable();
            self.text_input.commit();
            self.enabled = false;
        }
        if let Some(layer) = self.layer.take() {
            layer.destroy();
        }
        if let Some(surface) = self.surface.take() {
            surface.destroy();
        }
        self.entered = false;
        self.preedit.clear();
        self.pending_preedit = None;
        self.pending_commit = None;
        self.pending_delete = None;
    }

    fn on_key(&mut self, code: u32) {
        let Some(st) = &self.xkb_state else { return };
        let kc = xkb::Keycode::new(code);
        let sym = st.key_get_one_sym(kc).raw();
        let ctrl = st.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE);
        let shift = st.mod_name_is_active(xkb::MOD_NAME_SHIFT, xkb::STATE_MODS_EFFECTIVE);
        let mut meta = 0;
        if ctrl {
            meta |= META_CTRL;
        }
        if shift {
            meta |= META_SHIFT;
        }
        let android = match sym {
            0xff08 => Some(67),          // BackSpace -> KEYCODE_DEL
            0xff09 | 0xfe20 => Some(61), // Tab, ISO_Left_Tab
            0xff0d | 0xff8d => Some(66), // Return, KP_Enter
            0xff1b => Some(111),         // Escape
            0xffff => Some(112),         // Delete -> KEYCODE_FORWARD_DEL
            0xff50 => Some(122),         // Home -> KEYCODE_MOVE_HOME
            0xff57 => Some(123),         // End -> KEYCODE_MOVE_END
            0xff51 => Some(21),          // Left
            0xff52 => Some(19),          // Up
            0xff53 => Some(22),          // Right
            0xff54 => Some(20),          // Down
            0xff55 => Some(92),          // Page_Up
            0xff56 => Some(93),          // Page_Down
            _ => None,
        };
        if let Some(k) = android {
            self.out.push(format!("KEY {k} {meta}"));
            return;
        }
        if ctrl {
            let lower = sym | 0x20;
            if (0x61..=0x7a).contains(&lower) {
                self.out
                    .push(format!("KEY {} {meta}", KEYCODE_A + (lower - 0x61)));
            }
            return;
        }
        let text = st.key_get_utf8(kc);
        if !text.is_empty() && !text.chars().any(char::is_control) {
            self.out.push(format!("COMMIT {}", escape(&text)));
        }
    }

    fn text_input_done(&mut self) {
        let delete = self.pending_delete.take();
        let commit = self.pending_commit.take();
        let preedit = self.pending_preedit.take().unwrap_or_default();
        if let Some((before, after)) = delete {
            if before > 0 || after > 0 {
                self.out.push(format!("DELETE {before} {after}"));
            }
        }
        if let Some(text) = commit {
            self.out.push(format!("COMMIT {}", escape(&text)));
            self.preedit.clear();
        }
        if preedit != self.preedit {
            self.out.push(format!("PREEDIT {}", escape(&preedit)));
            self.preedit = preedit;
        }
    }

    /// Takes the keyboard focus back by mapping a new surface.
    fn regrab(&mut self, qh: &QueueHandle<App>) {
        let now = Instant::now();
        self.regrabs.retain(|t| now.duration_since(*t) < REGRAB_WINDOW);
        if self.regrabs.len() >= MAX_REGRABS {
            debug(format_args!("not taking the focus back again"));
            return;
        }
        self.regrabs.push(now);
        debug(format_args!("taking the keyboard focus back"));
        let content = self.content;
        let hide_at = self.hide_at;
        self.hide();
        self.show(qh, content.0, content.1);
        self.hide_at = hide_at;
    }

    fn toplevel_done(&mut self, id: &ObjectId, qh: &QueueHandle<App>) {
        let Some(t) = self.toplevels.get(id) else { return };
        debug(format_args!(
            "toplevel {:?} {:?} activated={} surface={} entered={}",
            id,
            t.app_id,
            t.activated,
            self.surface.is_some(),
            self.entered
        ));
        if is_waydroid(&t.app_id) {
            // The relaunched activity's window appeared.
            self.orphan_at = None;
        }
        // A Waydroid window took the focus from us while the keyboard is
        // wanted, e.g. a relaunched activity or an app's search screen.
        if self.surface.is_some() && !self.entered && t.activated && is_waydroid(&t.app_id) {
            self.regrab(qh);
            return;
        }
        // Holding the keyboard focus deactivates the Waydroid window, so only
        // another app coming to the front means the user left Android.
        if self.surface.is_some() && t.activated && !is_waydroid(&t.app_id) {
            self.hide();
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
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

delegate_noop!(App: ignore wl_compositor::WlCompositor);
delegate_noop!(App: ignore wl_shm::WlShm);
delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
delegate_noop!(App: ignore wl_buffer::WlBuffer);
delegate_noop!(App: ignore wl_region::WlRegion);
delegate_noop!(App: ignore wl_surface::WlSurface);
delegate_noop!(App: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(App: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        app: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Keyboard) && app.keyboard.is_none() {
                app.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                let keymap = unsafe {
                    xkb::Keymap::new_from_fd(
                        &app.xkb_ctx,
                        fd,
                        size as usize,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                };
                app.xkb_state = match keymap {
                    Ok(Some(k)) => Some(xkb::State::new(&k)),
                    _ => {
                        eprintln!("waydroid-ime-bridge: failed to load the keymap");
                        None
                    }
                };
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(st) = app.xkb_state.as_mut() {
                    st.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } => app.on_key(key + 8),
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for App {
    fn event(
        app: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer.ack_configure(serial);
                let (w, h) = (width as i32, height as i32);
                if w > 0 && h > 0 {
                    if w != app.full.0 {
                        app.full = (w, h);
                    } else {
                        app.full.1 = app.full.1.max(h);
                    }
                    let inset = app.full.1 - h;
                    if inset != app.inset {
                        debug(format_args!("keyboard inset {inset} of {w}x{}", app.full.1));
                        app.inset = inset;
                        app.out.push(format!("INSET {inset} {w}"));
                    }
                }
                if let Some(surface) = &app.surface {
                    surface.attach(Some(&app.buffer), 0, 0);
                    surface.damage_buffer(0, 0, 1, 1);
                    surface.commit();
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                debug(format_args!("layer surface closed by the compositor"));
                app.hide();
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for App {
    fn event(
        app: &mut Self,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_text_input_v3::Event;
        match event {
            Event::Enter { surface } => {
                debug(format_args!("text-input enter"));
                if app.surface.as_ref() == Some(&surface) {
                    app.entered = true;
                    app.enable();
                }
            }
            Event::Leave { .. } => {
                debug(format_args!("text-input leave"));
                app.entered = false;
                if app.enabled {
                    app.text_input.disable();
                    app.text_input.commit();
                    app.enabled = false;
                }
            }
            Event::PreeditString { text, .. } => app.pending_preedit = Some(text.unwrap_or_default()),
            Event::CommitString { text } => app.pending_commit = text,
            Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => app.pending_delete = Some((before_length, after_length)),
            Event::Done { .. } => app.text_input_done(),
            _ => {}
        }
    }
}

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            app.toplevels.insert(toplevel.id(), Toplevel::default());
        }
    }

    event_created_child!(App, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for App {
    fn event(
        app: &mut Self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event;
        let id = handle.id();
        match event {
            Event::AppId { app_id } => {
                app.toplevels.entry(id).or_default().app_id = app_id;
            }
            Event::State { state } => {
                let activated = state
                    .chunks_exact(4)
                    .any(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]) == 2);
                app.toplevels.entry(id).or_default().activated = activated;
            }
            Event::Done => app.toplevel_done(&id, qh),
            Event::Closed => {
                debug(format_args!("toplevel {id:?} closed"));
                app.toplevels.remove(&id);
                handle.destroy();
                let any_waydroid = app.toplevels.values().any(|t| is_waydroid(&t.app_id));
                if app.surface.is_some() && !any_waydroid && app.orphan_at.is_none() {
                    debug(format_args!("last Waydroid window closed"));
                    app.orphan_at = Some(Instant::now() + ORPHAN_DELAY);
                }
            }
            _ => {}
        }
    }
}

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    authed: bool,
}

fn token_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    base.join("waydroid-plinius/ime-token")
}

/// Trust on first use: the first input method that connects defines the
/// token; later clients must present the same one.
fn check_token(token: &str) -> bool {
    if token.len() < 16 {
        return false;
    }
    let path = token_path();
    match std::fs::read_to_string(&path) {
        Ok(known) => known.trim() == token,
        Err(_) => {
            use std::os::unix::fs::OpenOptionsExt;
            if let Some(dir) = path.parent() {
                use std::os::unix::fs::DirBuilderExt;
                let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
            }
            let saved = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .and_then(|mut f| f.write_all(token.as_bytes()));
            if let Err(e) = saved {
                eprintln!("waydroid-ime-bridge: cannot save {}: {e}", path.display());
            }
            true
        }
    }
}

/// Returns false when the client must be dropped.
fn handle_line(app: &mut App, qh: &QueueHandle<App>, client: &mut Client, line: &str) -> bool {
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    if !client.authed {
        if cmd == "HELLO" && check_token(arg.trim()) {
            client.authed = true;
            return true;
        }
        eprintln!("waydroid-ime-bridge: refused a client with an unknown token");
        return false;
    }
    debug(format_args!("client: {cmd} {arg}"));
    match cmd {
        "SHOW" => {
            let mut it = arg.split(' ').map(|v| v.parse::<u32>().unwrap_or(0));
            let hint = it.next().unwrap_or(0);
            let purpose = it.next().unwrap_or(0);
            app.show(qh, hint, purpose);
        }
        "HIDE" => {
            if app.surface.is_some() {
                app.hide_at = Some(Instant::now() + HIDE_DELAY);
            }
        }
        _ => {}
    }
    true
}

/// Reads what is available; returns false when the client is gone.
fn read_client(app: &mut App, qh: &QueueHandle<App>, client: &mut Client) -> bool {
    let mut chunk = [0u8; 4096];
    loop {
        match client.stream.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => client.buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
    while let Some(pos) = client.buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = client.buf.drain(..=pos).collect();
        let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
        if !handle_line(app, qh, client, line.trim_end_matches('\r')) {
            return false;
        }
    }
    client.buf.len() <= MAX_LINE
}

fn create_buffer(shm: &wl_shm::WlShm, qh: &QueueHandle<App>) -> wl_buffer::WlBuffer {
    let raw = unsafe { libc::memfd_create(c"waydroid-ime-bridge".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(raw >= 0, "memfd_create failed");
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // One transparent ARGB pixel; the new file is already zeroed.
    assert!(unsafe { libc::ftruncate(fd.as_raw_fd(), 4) } == 0, "ftruncate failed");
    let pool = shm.create_pool(fd.as_fd(), 4, qh, ());
    let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

fn main() {
    let addr: SocketAddr = std::env::var("WAYDROID_IME_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| DEFAULT_ADDR.parse().unwrap());

    let conn = Connection::connect_to_env().expect("cannot connect to the Wayland display");
    let (globals, mut queue) = registry_queue_init::<App>(&conn).expect("registry");
    let qh = queue.handle();

    let compositor: wl_compositor::WlCompositor =
        globals.bind(&qh, 4..=6, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).expect("wl_seat");
    let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 =
        globals.bind(&qh, 1..=4, ()).expect("zwlr_layer_shell_v1");
    let ti_manager: zwp_text_input_manager_v3::ZwpTextInputManagerV3 =
        globals.bind(&qh, 1..=1, ()).expect("zwp_text_input_manager_v3");
    let toplevel_manager = globals
        .bind::<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, _, _>(
            &qh,
            1..=3,
            (),
        )
        .ok();

    let text_input = ti_manager.get_text_input(&seat, &qh, ());
    let buffer = create_buffer(&shm, &qh);

    let mut app = App {
        compositor,
        layer_shell,
        text_input,
        keyboard: None,
        buffer,
        surface: None,
        layer: None,
        entered: false,
        enabled: false,
        content: (0, 0),
        pending_preedit: None,
        pending_commit: None,
        pending_delete: None,
        preedit: String::new(),
        xkb_ctx: xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
        xkb_state: None,
        track_toplevels: toplevel_manager.is_some(),
        toplevels: HashMap::new(),
        out: Vec::new(),
        hide_at: None,
        orphan_at: None,
        regrabs: Vec::new(),
        full: (0, 0),
        inset: 0,
    };

    let mut listener: Option<TcpListener> = None;
    let mut next_bind = Instant::now();
    let mut clients: Vec<Client> = Vec::new();

    loop {
        queue.dispatch_pending(&mut app).expect("Wayland dispatch");

        if !app.out.is_empty() {
            let lines = std::mem::take(&mut app.out);
            if let Some(i) = clients.iter().rposition(|c| c.authed) {
                let mut data = lines.join("\n");
                data.push('\n');
                // Block for the write; the lines are short and the peer is local.
                let s = &mut clients[i].stream;
                let sent = s
                    .set_nonblocking(false)
                    .and_then(|_| s.write_all(data.as_bytes()))
                    .and_then(|_| s.set_nonblocking(true));
                if sent.is_err() {
                    clients.remove(i);
                    app.hide();
                }
            }
        }
        if let Err(e) = conn.flush() {
            if !matches!(&e, WaylandError::Io(io) if io.kind() == ErrorKind::WouldBlock) {
                panic!("Wayland connection lost: {e}");
            }
        }

        let now = Instant::now();
        if listener.is_none() && now >= next_bind {
            match TcpListener::bind(addr) {
                Ok(l) => {
                    l.set_nonblocking(true).expect("nonblocking listener");
                    listener = Some(l);
                }
                Err(_) => next_bind = now + BIND_RETRY,
            }
        }

        let Some(guard) = queue.prepare_read() else { continue };

        let mut fds = vec![libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        if let Some(l) = &listener {
            fds.push(libc::pollfd { fd: l.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let first_client = fds.len();
        for c in &clients {
            fds.push(libc::pollfd { fd: c.stream.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }

        let mut deadline: Option<Instant> = match (app.hide_at, app.orphan_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if listener.is_none() {
            deadline = Some(deadline.map_or(next_bind, |d| d.min(next_bind)));
        }
        let timeout = deadline.map_or(-1, |d| {
            d.saturating_duration_since(Instant::now()).as_millis().min(60_000) as i32
        });

        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if n < 0 {
            drop(guard);
            continue;
        }

        if fds[0].revents != 0 {
            if let Err(e) = guard.read() {
                if !matches!(&e, WaylandError::Io(io) if io.kind() == ErrorKind::WouldBlock) {
                    panic!("Wayland connection lost: {e}");
                }
            }
        } else {
            drop(guard);
        }
        queue.dispatch_pending(&mut app).expect("Wayland dispatch");

        // Existing clients first: the indices in fds refer to them.
        let mut keep = Vec::with_capacity(clients.len());
        for (i, mut c) in std::mem::take(&mut clients).into_iter().enumerate() {
            let ready = fds[first_client + i].revents != 0;
            let was_authed = c.authed;
            if !ready || read_client(&mut app, &qh, &mut c) {
                if c.authed && !was_authed {
                    // A reconnecting input method replaces the old link.
                    keep.retain(|k: &Client| !k.authed);
                }
                keep.push(c);
            } else if c.authed {
                app.hide();
            }
        }
        clients = keep;

        if let Some(l) = &listener {
            if fds[1].revents != 0 {
                while let Ok((stream, _)) = l.accept() {
                    if stream.set_nonblocking(true).is_ok() {
                        let _ = stream.set_nodelay(true);
                        clients.push(Client { stream, buf: Vec::new(), authed: false });
                    }
                }
            }
        }

        let now = Instant::now();
        if app.hide_at.is_some_and(|t| now >= t) || app.orphan_at.is_some_and(|t| now >= t) {
            app.hide();
        }
    }
}
