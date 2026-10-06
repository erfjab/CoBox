//! Non-Windows stand-in so the UI can be built and tried on other OSes.
use crate::store::{Kind, NewClip};
use futures::channel::mpsc::UnboundedSender;
use std::{path::PathBuf, sync::atomic::AtomicBool};

#[allow(dead_code)]
pub enum Event { Clip(NewClip), Toggle }
pub static SKIP_PRIVATE: AtomicBool = AtomicBool::new(true);
pub static DIM_ALPHA: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(90);
pub const HOTKEYS: [(&str, u32, u32); 5] = [("alt + v", 0, 0), ("ctrl + shift + v", 0, 0), ("ctrl + alt + v", 0, 0), ("alt + c", 0, 0), ("ctrl + `", 0, 0)];
pub fn single_instance() {}
pub fn set_hotkey(_: usize) {}
pub fn spawn(_: UnboundedSender<Event>, _: PathBuf) {}
pub fn write(_: isize, _: Kind, _: &str, _: bool) {}
pub fn paste_into(_: isize) {}
pub fn foreground() -> isize { 0 }
pub fn show(_: isize) {}
pub fn hide(_: isize) {}
pub fn hwnd_of<T>(_: &T) -> isize { 0 }
pub fn autostart(_: bool) {}
pub fn utc_offset() -> i64 { 0 }
