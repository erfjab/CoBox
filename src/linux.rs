//! Linux: X11 (XWayland too) for clipboard change events, the global hotkey and simulated
//! Ctrl+V. The X server *pushes* clipboard owner changes (XFixes) and hotkey presses to a
//! thread that sleeps in `wait_for_event`, so nothing runs while idle.
//!
//! Without any X server (pure Wayland) the clipboard is checked once a second instead, and
//! the window is opened by running `cobox` again from a system keyboard shortcut.

use super::{Event, send, with_clip};
use crate::store::hash;
use std::{
    path::PathBuf,
    sync::{OnceLock, atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed}, mpsc::Sender},
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{Event as XEvent, xfixes::{self, ConnectionExt as _}, xproto::*, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

/// Hotkeys offered in settings: (label, X modifier mask, keysym).
pub const HOTKEYS: [(&str, u32, u32); 5] = [
    ("alt + v", ALT, XK_V),
    ("ctrl + shift + v", CTRL | SHIFT, XK_V),
    ("ctrl + alt + v", CTRL | ALT, XK_V),
    ("alt + c", ALT, XK_C),
    ("ctrl + `", CTRL, XK_GRAVE),
];
const SHIFT: u32 = 1;
const CTRL: u32 = 4;
const ALT: u32 = 8;
const XK_V: u32 = 0x76;
const XK_C: u32 = 0x63;
const XK_GRAVE: u32 = 0x60;
const XK_CONTROL_L: u32 = 0xffe3;

struct X {
    conn: RustConnection,
    root: Window,
    /// Hidden window that receives clipboard events and selection replies.
    win: Window,
    clipboard: Atom,
    targets: Atom,
    prop: Atom,
    /// KDE's marker for password-manager copies.
    secret: Atom,
    active: Atom,
}

static X11: OnceLock<Option<X>> = OnceLock::new();
static HOTKEY: AtomicUsize = AtomicUsize::new(0);
static PRIVATE: AtomicBool = AtomicBool::new(false);

fn x() -> Option<&'static X> { X11.get_or_init(|| connect().ok()).as_ref() }

fn connect() -> Result<X, Box<dyn std::error::Error>> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| -> Result<Atom, Box<dyn std::error::Error>> { Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom) };
    let (clipboard, targets, prop, secret, active) =
        (atom("CLIPBOARD")?, atom("TARGETS")?, atom("COBOX_TARGETS")?, atom("x-kde-passwordManagerHint")?, atom("_NET_ACTIVE_WINDOW")?);
    let win = conn.generate_id()?;
    conn.create_window(COPY_DEPTH_FROM_PARENT, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new())?;
    conn.xfixes_query_version(5, 0)?.reply()?;
    conn.xfixes_select_selection_input(win, clipboard, xfixes::SelectionEventMask::SET_SELECTION_OWNER)?;
    conn.flush()?;
    Ok(X { conn, root, win, clipboard, targets, prop, secret, active })
}

fn keycode(x: &X, sym: u32) -> Option<u8> {
    let s = x.conn.setup();
    let (min, max) = (s.min_keycode, s.max_keycode);
    let r = x.conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
    let per = (r.keysyms_per_keycode as usize).max(1);
    r.keysyms.chunks(per).position(|c| c.contains(&sym)).map(|i| min + i as u8)
}

fn grab(x: &X) {
    let (_, mods, sym) = HOTKEYS[HOTKEY.load(Relaxed).min(HOTKEYS.len() - 1)];
    x.conn.ungrab_key(Grab::ANY, x.root, ModMask::ANY).ok();
    if let Some(code) = keycode(x, sym) {
        // Also grab with Caps Lock / Num Lock on, or the hotkey stops working when they are.
        for locks in [0, 2, 16, 18] {
            x.conn.grab_key(true, x.root, ModMask::from((mods | locks) as u16), code, GrabMode::ASYNC, GrabMode::ASYNC).ok();
        }
    }
    x.conn.flush().ok();
}

/// Change the global hotkey (index into HOTKEYS).
pub fn set_hotkey(ix: usize) {
    HOTKEY.store(ix, Relaxed);
    if let Some(Some(x)) = X11.get() { grab(x) }
}

/// Tell `changed` every time something new is copied.
pub(super) fn watch(changed: Sender<()>) {
    let Some(x) = x() else { return poll(changed) };
    grab(x);
    thread::spawn(move || {
        let mut last_toggle = Instant::now() - Duration::from_secs(1);
        while let Ok(ev) = x.conn.wait_for_event() {
            match ev {
                // New owner: first ask which formats it offers, to spot password managers.
                XEvent::XfixesSelectionNotify(_) => {
                    x.conn.convert_selection(x.win, x.clipboard, x.targets, x.prop, CURRENT_TIME).ok();
                    x.conn.flush().ok();
                }
                XEvent::SelectionNotify(e) if e.selection == x.clipboard => {
                    let private = e.property != NONE
                        && x.conn.get_property(true, x.win, x.prop, AtomEnum::ATOM, 0, 1024).ok()
                            .and_then(|c| c.reply().ok())
                            .is_some_and(|r| r.value32().is_some_and(|mut v| v.any(|a| a == x.secret)));
                    PRIVATE.store(private, Relaxed);
                    if changed.send(()).is_err() { return; }
                }
                // Holding the keys repeats the press: ignore the repeats.
                XEvent::KeyPress(_) if last_toggle.elapsed() > Duration::from_millis(300) => {
                    last_toggle = Instant::now();
                    send(Event::Toggle);
                }
                _ => {}
            }
        }
    });
}

/// No X server: compare the clipboard with what it was a second ago.
fn poll(changed: Sender<()>) {
    let fingerprint = || with_clip(|c| {
        let text = c.get_text().unwrap_or_default();
        let files = c.get().file_list().unwrap_or_default();
        hash(text.as_bytes()) ^ hash(format!("{files:?}").as_bytes())
    });
    thread::spawn(move || {
        let mut last = fingerprint();
        loop {
            thread::sleep(Duration::from_secs(1));
            let now = fingerprint();
            if now != last {
                last = now;
                if changed.send(()).is_err() { return; }
            }
        }
    });
}

pub(super) fn is_private() -> bool { PRIVATE.load(Relaxed) }

fn active_window(x: &X) -> Window {
    x.conn.get_property(false, x.root, x.active, AtomEnum::WINDOW, 0, 1).ok()
        .and_then(|c| c.reply().ok())
        .and_then(|r| r.value32().and_then(|mut v| v.next()))
        .unwrap_or(0)
}

/// Name of the app in front when the copy happened ("firefox", "Code"…), from WM_CLASS.
pub(super) fn source_app() -> String {
    let Some(x) = x() else { return String::new() };
    let w = active_window(x);
    if w == 0 { return String::new(); }
    let Some(r) = x.conn.get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok().and_then(|c| c.reply().ok()) else {
        return String::new();
    };
    // "instance\0Class\0"
    r.value.split(|&b| b == 0).filter(|s| !s.is_empty()).last().map(|s| String::from_utf8_lossy(s).into_owned()).unwrap_or_default()
}

pub fn foreground() -> isize { x().map_or(0, |x| active_window(x) as isize) }

/// Focus the window that was active before CoBox and press Ctrl+V in it (X11 apps only;
/// on pure Wayland the item is copied and the user pastes it).
pub fn paste_into(prev: isize) {
    thread::spawn(move || {
        let Some(x) = x() else { return };
        thread::sleep(Duration::from_millis(80));
        if prev != 0 && active_window(x) as isize != prev {
            let ev = ClientMessageEvent::new(32, prev as Window, x.active, [2, CURRENT_TIME, 0, 0, 0]);
            x.conn.send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev).ok();
            x.conn.flush().ok();
            thread::sleep(Duration::from_millis(80));
        }
        let (Some(ctrl), Some(v)) = (keycode(x, XK_CONTROL_L), keycode(x, XK_V)) else { return };
        for (kind, key) in [(KEY_PRESS_EVENT, ctrl), (KEY_PRESS_EVENT, v), (KEY_RELEASE_EVENT, v), (KEY_RELEASE_EVENT, ctrl)] {
            x.conn.xtest_fake_input(kind, key, CURRENT_TIME, x.root, 0, 0, 0).ok();
        }
        x.conn.flush().ok();
    });
}

/// Start at login on/off: an XDG autostart entry pointing at this executable.
pub fn autostart(on: bool) {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let Some(config) = var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config"))) else { return };
    let path = config.join("autostart/cobox.desktop");
    if !on {
        std::fs::remove_file(path).ok();
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    super::write_if_changed(&path, &format!(
        "[Desktop Entry]\nType=Application\nName=CoBox\nComment=Clipboard history\nExec=\"{}\"\nIcon=cobox\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    ));
}
