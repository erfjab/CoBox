//! Everything Windows-specific: clipboard listener, global hotkey, writing the
//! clipboard back, simulated Ctrl+V, shell thumbnails, show/hide.
//!
//! One background thread owns a hidden message-only window. Windows *pushes*
//! WM_CLIPBOARDUPDATE / WM_HOTKEY to it, so the app uses no CPU while idle.

use crate::store::{Kind, NewClip, hash};
use futures::channel::mpsc::UnboundedSender;
use std::{ffi::c_void, path::{Path, PathBuf}, sync::{OnceLock, atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering::Relaxed}}, thread, time::Duration};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, DataExchange::*, LibraryLoader::GetModuleHandleW, Memory::*, Ole::CF_HDROP, Registry::*, Threading::*, Time::*},
        UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, PWSTR, w},
};

pub enum Event {
    Clip(NewClip),
    Toggle,
}

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;

static TX: OnceLock<(UnboundedSender<Event>, PathBuf)> = OnceLock::new();
static LISTENER: AtomicIsize = AtomicIsize::new(0);
/// Full-screen, click-through black layer shown behind CoBox.
static DIM: AtomicIsize = AtomicIsize::new(0);
/// Its opacity, 0–255 (0 = off).
pub static DIM_ALPHA: AtomicU32 = AtomicU32::new(90);
static HOTKEY: AtomicU32 = AtomicU32::new(0);
/// Skip clips that password managers mark as private.
pub static SKIP_PRIVATE: AtomicBool = AtomicBool::new(true);
const WM_SET_HOTKEY: u32 = WM_APP + 1;

/// Hotkeys offered in settings: (label, modifiers, virtual key).
pub const HOTKEYS: [(&str, u32, u32); 5] = [
    ("alt + v", 0x1, 0x56),
    ("ctrl + shift + v", 0x2 | 0x4, 0x56),
    ("ctrl + alt + v", 0x1 | 0x2, 0x56),
    ("alt + c", 0x1, 0x43),
    ("ctrl + `", 0x2, 0xC0),
];

/// Only one CoBox at a time: a second launch exits immediately.
pub fn single_instance() {
    unsafe {
        let _ = CreateMutexW(None, true, w!("CoBox.single-instance"));
        if GetLastError() == ERROR_ALREADY_EXISTS { std::process::exit(0); }
    }
}

/// Change the global hotkey (index into HOTKEYS).
pub fn set_hotkey(ix: usize) {
    HOTKEY.store(ix as u32, Relaxed);
    let h = LISTENER.load(Relaxed);
    if h != 0 {
        unsafe { PostMessageW(Some(HWND(h as *mut c_void)), WM_SET_HOTKEY, WPARAM(0), LPARAM(0)).ok(); }
    }
}

unsafe fn register_hotkey(hwnd: HWND) {
    let (_, m, vk) = HOTKEYS[(HOTKEY.load(Relaxed) as usize).min(HOTKEYS.len() - 1)];
    unsafe {
        UnregisterHotKey(Some(hwnd), 1).ok();
        RegisterHotKey(Some(hwnd), 1, HOT_KEY_MODIFIERS(m) | MOD_NOREPEAT, vk).ok();
    }
}

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() }

/// Start the listener thread. `dir` is where image/thumbnail pngs go.
pub fn spawn(tx: UnboundedSender<Event>, dir: PathBuf) {
    TX.set((tx, dir)).ok();
    thread::spawn(|| unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok();
        let inst = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let class = w!("cobox_listener");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
        let hwnd = CreateWindowExW(Default::default(), class, w!(""), Default::default(), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None).unwrap();
        AddClipboardFormatListener(hwnd).ok();
        // Alt+V opens CoBox (Win+V belongs to Windows).
        LISTENER.store(hwnd.0 as isize, Relaxed);

        let dim_class = w!("cobox_dim");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(dim_proc),
            hInstance: inst.into(),
            lpszClassName: dim_class,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        });
        if let Ok(dim) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            dim_class, w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(inst.into()), None,
        ) {
            DIM.store(dim.0 as isize, Relaxed);
        }
        register_hotkey(hwnd);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
}

unsafe extern "system" fn dim_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some((tx, dir)) = TX.get() else { return unsafe { DefWindowProcW(hwnd, msg, wp, lp) } };
    match msg {
        WM_CLIPBOARDUPDATE => {
            if let Some(c) = unsafe { read_clipboard(hwnd, dir) } {
                tx.unbounded_send(Event::Clip(c)).ok();
            }
            LRESULT(0)
        }
        WM_SET_HOTKEY => {
            unsafe { register_hotkey(hwnd) };
            LRESULT(0)
        }
        WM_HOTKEY => {
            tx.unbounded_send(Event::Toggle).ok();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// The clipboard is often briefly locked by the app that just wrote it.
unsafe fn open_clipboard(hwnd: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() { return true; }
        thread::sleep(Duration::from_millis(15));
    }
    false
}

unsafe fn read_clipboard(hwnd: HWND, dir: &Path) -> Option<NewClip> {
    unsafe {
        // Respect password managers (same convention Windows' own history uses).
        let skip = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let allow = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        let src = source_app();
        if !open_clipboard(hwnd) { return None; }
        let private = SKIP_PRIVATE.load(Relaxed) && IsClipboardFormatAvailable(skip).is_ok()
            || SKIP_PRIVATE.load(Relaxed) && (IsClipboardFormatAvailable(allow).is_ok() && with_global(allow, |p, _| *(p as *const u32)) == Some(0));
        if private {
            CloseClipboard().ok();
            return None;
        }
        let clip = if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
            read_files(dir, src)
        } else if IsClipboardFormatAvailable(CF_DIB).is_ok() {
            read_dib(dir, src)
        } else if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            read_text(src)
        } else {
            None
        };
        CloseClipboard().ok();
        clip
    }
}

unsafe fn with_global<R>(fmt: u32, f: impl FnOnce(*const u8, usize) -> R) -> Option<R> {
    unsafe {
        let h = HGLOBAL(GetClipboardData(fmt).ok()?.0);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() { return None; }
        let r = f(p, GlobalSize(h));
        GlobalUnlock(h).ok();
        Some(r)
    }
}

unsafe fn read_text(src: String) -> Option<NewClip> {
    let text = unsafe {
        with_global(CF_UNICODETEXT, |p, n| {
            let s = std::slice::from_raw_parts(p as *const u16, n / 2);
            String::from_utf16_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
        })?
    };
    if text.trim().is_empty() { return None; }
    let lines = text.lines().count();
    Some(NewClip {
        kind: Kind::of_text(&text),
        meta: if lines > 1 { format!("{lines} lines") } else { format!("{} chars", text.chars().count()) },
        hash: hash(text.as_bytes()),
        size: text.len() as i64,
        thumb: String::new(),
        text,
        src,
    })
}

unsafe fn read_files(dir: &Path, src: String) -> Option<NewClip> {
    unsafe {
        let hdrop = HDROP(GetClipboardData(CF_HDROP.0 as u32).ok()?.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        let mut paths = Vec::new();
        for i in 0..n {
            let mut buf = vec![0u16; DragQueryFileW(hdrop, i, None) as usize + 1];
            let len = DragQueryFileW(hdrop, i, Some(buf.as_mut_slice())) as usize;
            paths.push(String::from_utf16_lossy(&buf[..len]));
        }
        let first = paths.first()?.clone();
        let text = paths.join("\n");
        let h = hash(text.as_bytes());
        let kind = if n == 1 { Kind::of_path(&first) } else { Kind::File };
        let size = paths.iter().map(|p| std::fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0)).sum();
        let meta = if n == 1 { first.clone() } else { format!("{n} files") };
        Some(NewClip { kind, meta, thumb: shell_thumb(&first, dir, h).unwrap_or_default(), text, src, size, hash: h })
    }
}

/// CF_DIB (any 24/32-bit bitmap) → png file.
unsafe fn read_dib(dir: &Path, src: String) -> Option<NewClip> {
    let (w, h, rgba) = unsafe {
        with_global(CF_DIB, |p, _| {
            let hdr = &*(p as *const BITMAPINFOHEADER);
            let (w, h, bpp) = (hdr.biWidth, hdr.biHeight, hdr.biBitCount as usize);
            if w <= 0 || h == 0 || (bpp != 24 && bpp != 32) { return None; }
            let masks = if hdr.biCompression == 3 && hdr.biSize == 40 { 12 } else { 0 };
            let px = p.add(hdr.biSize as usize + masks + hdr.biClrUsed as usize * 4);
            Some((w as u32, h.unsigned_abs(), dib_to_rgba(px, w as usize, h, bpp)))
        })??
    };
    let hs = hash(&rgba);
    let path = dir.join(format!("{:016x}.png", hs as u64));
    if !path.exists() {
        image::save_buffer(&path, &rgba, w, h, image::ExtendedColorType::Rgba8).ok()?;
    }
    let p = path.to_string_lossy().into_owned();
    let size = std::fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
    Some(NewClip { kind: Kind::Image, meta: format!("{w}×{h}"), thumb: p.clone(), text: p, src, size, hash: hs })
}

/// Bottom-up (h>0) or top-down (h<0) BGR(A) rows → top-down RGBA.
unsafe fn dib_to_rgba(px: *const u8, w: usize, h: i32, bpp: usize) -> Vec<u8> {
    let rows = h.unsigned_abs() as usize;
    let stride = (w * bpp / 8 + 3) & !3;
    let mut out = Vec::with_capacity(w * rows * 4);
    let mut any_alpha = false;
    for y in 0..rows {
        let sy = if h > 0 { rows - 1 - y } else { y };
        let row = unsafe { std::slice::from_raw_parts(px.add(sy * stride), w * bpp / 8) };
        for c in row.chunks_exact(bpp / 8) {
            let a = if bpp == 32 { c[3] } else { 255 };
            any_alpha |= a != 0;
            out.extend_from_slice(&[c[2], c[1], c[0], a]);
        }
    }
    if !any_alpha {
        out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    out
}

/// Windows already knows how to thumbnail videos, pdfs and images — ask it.
unsafe fn shell_thumb(path: &str, dir: &Path, h: i64) -> Option<String> {
    unsafe {
        let out = dir.join(format!("t{:016x}.png", h as u64));
        if out.exists() { return Some(out.to_string_lossy().into_owned()); }
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(wide(path).as_ptr()), None).ok()?;
        let hbmp = f.GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(hbmp.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void));
        let (w, hh) = (bm.bmWidth, bm.bmHeight);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -hh, biPlanes: 1, biBitCount: 32, ..Default::default() };
        let mut buf = vec![0u8; (w * hh * 4) as usize];
        let dc = GetDC(None);
        GetDIBits(dc, hbmp, 0, hh as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        DeleteObject(hbmp.into()).ok().ok();
        let rgba = dib_to_rgba(buf.as_ptr(), w as usize, -hh, 32);
        image::save_buffer(&out, &rgba, w as u32, hh as u32, image::ExtendedColorType::Rgba8).ok()?;
        Some(out.to_string_lossy().into_owned())
    }
}

/// Name of the app in the foreground when the copy happened ("chrome", "Code"…).
fn source_app() -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid as *mut u32));
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        CloseHandle(h).ok();
        if !ok { return String::new(); }
        let p = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}


// ───────────────────────────── writing / pasting ─────────────────────────────

unsafe fn set_data(fmt: u32, bytes: &[u8]) {
    unsafe {
        let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else { return };
        let p = GlobalLock(h) as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h).ok();
        SetClipboardData(fmt, Some(HANDLE(h.0))).ok();
    }
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    wide(s).iter().flat_map(|c| c.to_le_bytes()).collect()
}

/// Put an item back on the clipboard. `owner` must be one of our windows.
pub fn write(owner: isize, kind: Kind, text: &str, plain: bool) {
    unsafe {
        if !open_clipboard(HWND(owner as *mut c_void)) { return; }
        EmptyClipboard().ok();
        let ours = TX.get().is_some_and(|t| Path::new(text).starts_with(&t.1));
        if plain || matches!(kind, Kind::Text | Kind::Code | Kind::Link) {
            set_data(CF_UNICODETEXT, &utf16_bytes(text));
        } else if !(kind == Kind::Image && ours) {
            // DROPFILES header (20 bytes, wide paths) + double-null-terminated list.
            let mut b = vec![20u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0];
            for p in text.lines() { b.extend(utf16_bytes(p)); }
            b.extend([0, 0]);
            set_data(CF_HDROP.0 as u32, &b);
        } else if let Ok(img) = image::open(text) {
            let img = img.to_rgba8();
            let (w, h) = img.dimensions();
            let hdr = BITMAPINFOHEADER { biSize: 40, biWidth: w as i32, biHeight: -(h as i32), biPlanes: 1, biBitCount: 32, ..Default::default() };
            let mut b = std::slice::from_raw_parts(&hdr as *const _ as *const u8, 40).to_vec();
            b.extend(img.pixels().flat_map(|p| [p[2], p[1], p[0], p[3]]));
            set_data(CF_DIB, &b);
        }
        CloseClipboard().ok();
    }
}

/// Focus the window that was active before CoBox and press Ctrl+V in it.
pub fn paste_into(prev: isize) {
    thread::spawn(move || unsafe {
        SetForegroundWindow(HWND(prev as *mut c_void)).ok().ok();
        thread::sleep(Duration::from_millis(40));
        let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
        };
        let seq = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
        SendInput(&seq, size_of::<INPUT>() as i32);
    });
}

pub fn foreground() -> isize { unsafe { GetForegroundWindow().0 as isize } }

pub fn show(hwnd: isize) {
    unsafe {
        let me = HWND(hwnd as *mut c_void);
        ShowWindow(me, SW_SHOW).ok().ok();
        let dim = HWND(DIM.load(Relaxed) as *mut c_void);
        let alpha = DIM_ALPHA.load(Relaxed);
        if !dim.0.is_null() && alpha > 0 {
            SetLayeredWindowAttributes(dim, COLORREF(0), alpha as u8, LWA_ALPHA).ok();
            // Cover every monitor, right behind CoBox.
            let (x, y) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
            SetWindowPos(dim, Some(me), x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW).ok();
        }
    }
}

pub fn hide(hwnd: isize) {
    unsafe {
        ShowWindow(HWND(hwnd as *mut c_void), SW_HIDE).ok().ok();
        let dim = DIM.load(Relaxed);
        if dim != 0 { ShowWindow(HWND(dim as *mut c_void), SW_HIDE).ok().ok(); }
    }
}

pub fn hwnd_of(w: &impl raw_window_handle::HasWindowHandle) -> isize {
    match w.window_handle().map(|h| h.as_raw()) {
        Ok(raw_window_handle::RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

/// Start with Windows on/off (HKCU\\...\\Run). Re-written each launch so moving the exe just works.
pub fn autostart(on: bool) {
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, w!("CoBox"));
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let v = wide(&format!("\"{}\"", exe.display()));
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, w!("CoBox"), REG_SZ.0, Some(v.as_ptr() as *const c_void), (v.len() * 2) as u32);
    }
}

/// Local time offset from UTC in seconds.
pub fn utc_offset() -> i64 {
    unsafe {
        let mut tz = TIME_ZONE_INFORMATION::default();
        let daylight = GetTimeZoneInformation(&mut tz) == 2;
        -((tz.Bias + if daylight { tz.DaylightBias } else { 0 }) as i64) * 60
    }
}
