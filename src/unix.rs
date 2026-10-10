//! macOS and Linux: everything the two share. The clipboard is read and written through
//! `arboard`; watching it, the global hotkey, pasting and startup live in `mac.rs` / `linux.rs`.
//!
//! A second launch doesn't start another copy: it tells the running one to open, so a
//! system keyboard shortcut bound to `cobox` works where global hotkeys can't (Wayland).

use crate::store::{Kind, NewClip, hash};
use arboard::{Clipboard, ImageData};
use futures::channel::mpsc::UnboundedSender;
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, atomic::{AtomicBool, AtomicU32, Ordering::Relaxed}, mpsc},
    thread,
    time::Duration,
};

#[cfg(target_os = "macos")]
#[path = "mac.rs"]
mod os;
#[cfg(not(target_os = "macos"))]
#[path = "linux.rs"]
mod os;

pub use os::{HOTKEYS, autostart, foreground, paste_into, set_hotkey};

pub enum Event {
    Clip(NewClip),
    Toggle,
}

/// Skip clips that password managers mark as private.
pub static SKIP_PRIVATE: AtomicBool = AtomicBool::new(true);
/// Screen dimming is Windows-only; kept so the setting compiles everywhere.
pub static DIM_ALPHA: AtomicU32 = AtomicU32::new(90);

static TX: OnceLock<(UnboundedSender<Event>, PathBuf)> = OnceLock::new();
static LISTENER: OnceLock<UnixListener> = OnceLock::new();
/// Kept alive for the whole run: on Linux the app that set the clipboard has to serve it.
static CLIP: Mutex<Option<Clipboard>> = Mutex::new(None);

pub(crate) fn send(ev: Event) {
    if let Some((tx, _)) = TX.get() { tx.unbounded_send(ev).ok(); }
}

fn socket() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir());
    match dir {
        Some(d) => d.join("cobox.sock"),
        None => std::env::temp_dir().join(format!("cobox-{}.sock", unsafe { libc::getuid() })),
    }
}

/// Only one CoBox at a time: a second launch asks the first one to open, then exits.
pub fn single_instance() {
    let path = socket();
    if let Ok(mut s) = UnixStream::connect(&path) {
        s.write_all(b"toggle").ok();
        std::process::exit(0);
    }
    std::fs::remove_file(&path).ok();
    if let Ok(l) = UnixListener::bind(&path) { LISTENER.set(l).ok(); }
}

/// Start watching the clipboard. `dir` is where image/thumbnail pngs go.
pub fn spawn(tx: UnboundedSender<Event>, dir: PathBuf) {
    TX.set((tx, dir)).ok();
    if let Some(l) = LISTENER.get() {
        thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0u8; 16];
                if s.read(&mut buf).is_ok_and(|n| n > 0) { send(Event::Toggle) }
            }
        });
    }
    // The platform watcher only says "it changed"; reading happens here, off its thread.
    let (changed, rx) = mpsc::channel::<()>();
    os::watch(changed);
    thread::spawn(move || {
        while rx.recv().is_ok() {
            // Apps often write several formats in a row: wait for them to settle.
            thread::sleep(Duration::from_millis(60));
            while rx.try_recv().is_ok() {}
            if SKIP_PRIVATE.load(Relaxed) && os::is_private() { continue; }
            if let Some(c) = read() { send(Event::Clip(c)) }
        }
    });
}

fn with_clip<R>(f: impl FnOnce(&mut Clipboard) -> R) -> Option<R> {
    let mut g = CLIP.lock().ok()?;
    if g.is_none() { *g = Clipboard::new().ok(); }
    g.as_mut().map(f)
}

/// Read the clipboard: files first, then text, then a bitmap.
pub(crate) fn read() -> Option<NewClip> {
    let dir = &TX.get()?.1;
    let src = os::source_app();
    let files = with_clip(|c| c.get().file_list().ok()).flatten().filter(|f| !f.is_empty());
    if let Some(paths) = files {
        return read_files(paths, dir, src);
    }
    if let Some(text) = with_clip(|c| c.get_text().ok()).flatten().filter(|t| !t.trim().is_empty()) {
        let lines = text.lines().count();
        return Some(NewClip {
            kind: Kind::of_text(&text),
            meta: if lines > 1 { format!("{lines} lines") } else { format!("{} chars", text.chars().count()) },
            hash: hash(text.as_bytes()),
            size: text.len() as i64,
            thumb: String::new(),
            text,
            src,
        });
    }
    let img = with_clip(|c| c.get_image().ok()).flatten()?;
    let (w, h) = (img.width as u32, img.height as u32);
    let hs = hash(&img.bytes);
    let path = dir.join(format!("{:016x}.png", hs as u64));
    if !path.exists() {
        image::save_buffer(&path, &img.bytes, w, h, image::ExtendedColorType::Rgba8).ok()?;
    }
    let p = path.to_string_lossy().into_owned();
    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    Some(NewClip { kind: Kind::Image, meta: format!("{w}×{h}"), thumb: p.clone(), text: p, src, size, hash: hs })
}

fn read_files(paths: Vec<PathBuf>, dir: &Path, src: String) -> Option<NewClip> {
    let paths: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let first = paths.first()?.clone();
    let text = paths.join("\n");
    let h = hash(text.as_bytes());
    let n = paths.len();
    let kind = if n == 1 { Kind::of_path(&first) } else { Kind::File };
    let size = paths.iter().map(|p| std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0)).sum();
    let meta = if n == 1 { first.clone() } else { format!("{n} files") };
    let thumb = if kind == Kind::Image { thumbnail(&first, dir, h).unwrap_or_default() } else { String::new() };
    Some(NewClip { kind, meta, thumb, text, src, size, hash: h })
}

/// A 256px png of an image file, so big photos don't slow the list down.
fn thumbnail(path: &str, dir: &Path, h: i64) -> Option<String> {
    let out = dir.join(format!("t{:016x}.png", h as u64));
    if !out.exists() {
        if std::fs::metadata(path).ok()?.len() > 64 << 20 { return None; }
        image::open(path).ok()?.thumbnail(256, 256).save(&out).ok()?;
    }
    Some(out.to_string_lossy().into_owned())
}

/// Put an item back on the clipboard.
pub fn write(_owner: isize, kind: Kind, text: &str, plain: bool) {
    let ours = TX.get().is_some_and(|t| Path::new(text).starts_with(&t.1));
    with_clip(|c| {
        if plain || matches!(kind, Kind::Text | Kind::Code | Kind::Link) {
            c.set_text(text).ok();
        } else if !(kind == Kind::Image && ours) {
            c.set().file_list(&text.lines().collect::<Vec<_>>()).ok();
        } else if let Ok(img) = image::open(text) {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            c.set_image(ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() }).ok();
        }
    });
}

pub fn hwnd_of<T>(_: &T) -> isize { 0 }

/// Local time offset from UTC in seconds.
pub fn utc_offset() -> i64 {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_gmtoff as i64
    }
}

/// Write `body` to `path` only if it changed (startup entries are rewritten on every launch).
pub(crate) fn write_if_changed(path: &Path, body: &str) {
    if std::fs::read_to_string(path).is_ok_and(|old| old == body) { return; }
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).ok(); }
    std::fs::write(path, body).ok();
}
