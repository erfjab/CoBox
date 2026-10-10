#![cfg_attr(windows, windows_subsystem = "windows")]
//! CoBox — a tiny, keyboard-first clipboard history that lives in the background.

mod store;
/// Platform layer: clipboard, global hotkey, paste, startup. Same functions on every OS.
#[cfg(windows)]
#[path = "win.rs"]
mod sys;
#[cfg(not(windows))]
#[path = "unix.rs"]
mod sys;

use futures::StreamExt;
use gpui::{
    App, Application, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, HighlightStyle, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, ScrollStrategy, SharedString, StyledText, Subscription, Task, TextRun, UniformListScrollHandle,
    Window, WindowAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, img, prelude::*, px, rgb, size,
    uniform_list,
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};
use store::{Item, Kind, Store, human};

const FONT: &str = if cfg!(windows) { "Cascadia Mono" } else if cfg!(target_os = "macos") { "Menlo" } else { "DejaVu Sans Mono" };
const W: f32 = 640.;
const H: f32 = 460.;

// ─────────────────────────────── look ───────────────────────────────

struct Theme { bg: u32, panel: u32, fg: u32, dim: u32, faint: u32, line: u32 }
const DARK: Theme = Theme { bg: 0x16161a, panel: 0x1d1d22, fg: 0xe8e8ec, dim: 0x9a9aa6, faint: 0x5c5c68, line: 0x2c2c34 };
const LIGHT: Theme = Theme { bg: 0xfbfbfc, panel: 0xf2f2f4, fg: 0x1a1a1f, dim: 0x60606b, faint: 0xa6a6b0, line: 0xe2e2e7 };

/// fill, text-on-dark, text-on-light, text-on-fill, row-on-dark, row-on-light (text kept ≥ 4.5:1).
const ACCENTS: [[u32; 6]; 5] = [
    [0xffbe0b, 0xffbe0b, 0x8c6b13, 0x111111, 0x3a321a, 0xfff8e7], // amber
    [0xfb5607, 0xfb5607, 0xce4a0b, 0x111111, 0x3a2219, 0xffeee6], // flame
    [0xff006e, 0xff006e, 0xe80266, 0x111111, 0x3b1428, 0xffe6f0], // rose
    [0x8338ec, 0x9e66ee, 0x8338ec, 0xffffff, 0x281d3b, 0xf3ebfd], // violet
    [0x3a86ff, 0x3a86ff, 0x3370d1, 0x111111, 0x1d293f, 0xebf3ff], // azure (default)
];

/// The CoBox logo (three stacked clips) in the accent color, `h` px tall.
fn mark(h: f32, color: u32) -> gpui::Div {
    let k = h / 144.;
    let bar = |w: f32, bh: f32, o: f32| div().w(px(w * k)).h(px(bh * k)).rounded(px(bh * k / 3.5)).bg(rgb(color)).opacity(o);
    div().flex_none().flex().flex_col().items_center().gap(px(12. * k))
        .child(bar(112., 28., 0.3)).child(bar(132., 32., 0.6)).child(bar(152., 60., 1.))
}

const CATS: [&str; 5] = ["all", "text", "link", "media", "file"];
const DAYS: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

// ─────────────────────────────── settings ───────────────────────────────

const THEMES: [&str; 3] = ["auto", "dark", "light"];
const KEEP: [(&str, i64); 5] = [("forever", 0), ("1 year", 365), ("3 months", 90), ("1 month", 30), ("1 week", 7)];
const SIZES: [f32; 3] = [12., 13., 14.];
const ONOFF: [&str; 2] = ["on", "off"];
const DIM: [(&str, u32); 4] = [("off", 0), ("light", 70), ("medium", 120), ("dark", 170)];

/// One row per setting; values are option indices saved in the db as `s.<key>`.
const SETTINGS: [(&str, &str); 8] = [
    ("theme", "theme"),
    ("primary", "accent"),
    ("open with", "hotkey"),
    (if cfg!(windows) { "start with windows" } else { "start at login" }, "autostart"),
    ("keep history", "keep"),
    ("skip passwords", "private"),
    ("text size", "size"),
    ("dim screen", "dim"),
];
const DEFAULTS: [usize; 8] = [0, 4, 0, 0, 0, 0, 1, 1];

fn options(row: usize) -> Vec<&'static str> {
    match row {
        0 => THEMES.to_vec(),
        1 => vec![""; ACCENTS.len()],
        2 => sys::HOTKEYS.iter().map(|h| h.0).collect(),
        3 | 5 => ONOFF.to_vec(),
        4 => KEEP.iter().map(|k| k.0).collect(),
        6 => vec!["12", "13", "14"],
        _ => DIM.iter().map(|d| d.0).collect(),
    }
}

enum Row { Head(SharedString), Item(usize) }

struct CoBox {
    store: Store,
    items: Vec<Item>,
    /// Indices into `items` matching search + category, in display order.
    view: Vec<usize>,
    /// What the list draws: day headers + items. `row_of[vi]` = row index of view item `vi`.
    rows: Vec<Row>,
    row_of: Vec<usize>,
    query: String,
    cat: usize,
    sel: usize,
    wide: bool,
    /// Settings panel: `Some(selected row)` while open.
    settings: Option<usize>,
    prefs: [usize; 8],
    dark: bool,
    toast: Option<SharedString>,
    _toast_timer: Option<Task<()>>,
    undo: Option<Item>,
    hwnd: isize,
    prev: isize,
    tz: i64,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// The open window. On Windows it lives for the whole run and is hidden/shown; elsewhere
    /// hiding closes it and the next toggle opens a fresh one (Linux has no hidden windows).
    window: Option<WindowHandle<CoBox>>,
    _subs: Vec<Subscription>,
}

impl CoBox {
    fn new(store: Store, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let prefs: [usize; 8] = std::array::from_fn(|i| {
            store.get(&format!("s.{}", SETTINGS[i].1)).and_then(|v| v.parse().ok()).unwrap_or(DEFAULTS[i])
        });
        let mut this = Self {
            items: vec![],
            store,
            view: vec![], rows: vec![], row_of: vec![],
            query: String::new(), cat: 0, sel: 0, wide: false, settings: None,
            prefs, dark: true,
            toast: None, _toast_timer: None, undo: None,
            hwnd: 0, prev: 0, tz: sys::utc_offset(),
            focus, scroll: UniformListScrollHandle::new(),
            window: None, _subs: vec![],
        };
        for row in [2, 3, 4, 5, 7] { this.apply(row) }
        this.items = this.store.all();
        this.refilter();
        this
    }

    /// Hook up a newly opened window.
    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hwnd = sys::hwnd_of(window);
        let hwnd = self.hwnd;
        // Never quit: closing (Alt+F4) or clicking elsewhere just hides it.
        window.on_window_should_close(cx, move |_, cx| hide_window(hwnd, cx));
        self._subs = vec![
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() { this.hide(window, cx) }
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
    }

    fn t(&self) -> &'static Theme { if self.dark { &DARK } else { &LIGHT } }
    fn a(&self) -> [u32; 6] { ACCENTS[self.prefs[1]] }
    fn atext(&self) -> u32 { self.a()[if self.dark { 1 } else { 2 }] }
    fn fs(&self) -> f32 { SIZES[self.prefs[6]] }
    fn row_h(&self) -> f32 { (self.fs() * 2.1).round() }
    fn cur(&self) -> Option<&Item> { self.view.get(self.sel).map(|&i| &self.items[i]) }
    fn sort(&mut self) { self.items.sort_by_key(|i| (!i.pinned, -i.ts)); }

    // ─────────────── settings ───────────────

    fn set_pref(&mut self, row: usize, v: usize) {
        self.prefs[row] = v % options(row).len();
        self.store.set(&format!("s.{}", SETTINGS[row].1), &self.prefs[row].to_string());
        self.apply(row);
    }

    /// Side effects of a setting (the rest is read at render time).
    fn apply(&mut self, row: usize) {
        match row {
            2 => sys::set_hotkey(self.prefs[2]),
            3 => sys::autostart(self.prefs[3] == 0),
            4 => {
                let days = KEEP[self.prefs[4]].1;
                if days > 0 {
                    let img = data_dir().join("img");
                    for f in self.store.prune(days) {
                        if PathBuf::from(&f).starts_with(&img) { std::fs::remove_file(f).ok(); }
                    }
                    self.items = self.store.all();
                    self.refilter();
                }
            }
            5 => sys::SKIP_PRIVATE.store(self.prefs[5] == 0, Ordering::Relaxed),
            7 => sys::DIM_ALPHA.store(DIM[self.prefs[7]].1, Ordering::Relaxed),
            _ => {}
        }
    }

    // ─────────────── list ───────────────

    fn day(&self, ts: i64) -> i64 { (ts + self.tz).div_euclid(86400) }

    /// "today", "yesterday", "saturday", or a date for older days.
    fn day_label(&self, ts: i64) -> String {
        let (d, today) = (self.day(ts), self.day(store::now()));
        match today - d {
            0 => "today".into(),
            1 => "yesterday".into(),
            2..=6 => DAYS[(d + 4).rem_euclid(7) as usize].into(),
            _ => { let (y, m, dd) = ymd(d); format!("{y}-{m:02}-{dd:02}") }
        }
    }

    fn clock(&self, ts: i64) -> String {
        let s = (ts + self.tz).rem_euclid(86400);
        format!("{:02}:{:02}", s / 3600, s / 60 % 60)
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let cat = self.cat;
        self.view = self.items.iter().enumerate()
            .filter(|(_, i)| (cat == 0 || i.kind.group() == cat)
                && words.iter().all(|w| i.lc.contains(w) || i.src.to_lowercase().contains(w)))
            .map(|(ix, _)| ix)
            .collect();
        // Pinned first, then one header per day.
        self.rows.clear();
        self.row_of.clear();
        let mut last = String::new();
        for (vi, &ix) in self.view.iter().enumerate() {
            let it = &self.items[ix];
            let head = if it.pinned { "pinned".to_string() } else { self.day_label(it.ts) };
            if head != last {
                self.rows.push(Row::Head(head.clone().into()));
                last = head;
            }
            self.row_of.push(self.rows.len());
            self.rows.push(Row::Item(vi));
        }
        self.select(self.sel);
    }

    fn select(&mut self, vi: usize) {
        let down = vi > self.sel;
        self.sel = vi.min(self.view.len().saturating_sub(1));
        let Some(&r) = self.row_of.get(self.sel) else { return };
        // Scroll only as far as needed, one row at a time: going down the row sticks to the
        // bottom edge; going up, the row above it (often the day header) sticks to the top.
        if down {
            self.scroll.scroll_to_item(r, ScrollStrategy::Bottom);
        } else {
            self.scroll.scroll_to_item(r.saturating_sub(1), ScrollStrategy::Top);
        }
    }

    fn flash(&mut self, msg: &'static str, cx: &mut Context<Self>) {
        self.toast = Some(msg.into());
        self._toast_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1600)).await;
            this.update(cx, |this, cx| { this.toast = None; cx.notify(); }).ok();
        }));
    }

    // ─────────────── show / hide ───────────────

    /// `prev` is the window (or app) that was in front, where Enter pastes.
    fn show(&mut self, prev: isize, window: &mut Window) {
        self.prev = prev;
        self.query.clear();
        self.settings = None;
        self.wide = false;
        self.sel = 0;
        self.refilter();
        #[cfg(windows)]
        sys::show(self.hwnd);
        window.activate_window();
        window.focus(&self.focus);
    }

    fn hide(&mut self, window: &mut Window, cx: &mut App) {
        if !hide_window(self.hwnd, cx) { return }
        window.remove_window();
        self.window = None;
    }

    fn add(&mut self, c: store::NewClip, cx: &mut Context<Self>) {
        if let Some(item) = self.store.upsert(&c) {
            self.items.retain(|i| i.id != item.id);
            self.items.push(item);
            self.sort();
            self.refilter();
            cx.notify();
        }
    }

    // ─────────────── actions ───────────────

    fn paste(&mut self, plain: bool, window: &mut Window, cx: &mut App) {
        let Some(i) = self.cur() else { return };
        sys::write(self.hwnd, i.kind, &i.text, plain);
        self.hide(window, cx);
        sys::paste_into(self.prev);
    }

    fn pin(&mut self, cx: &mut Context<Self>) {
        let Some(&ix) = self.view.get(self.sel) else { return };
        let (id, pinned) = { let i = &mut self.items[ix]; i.pinned = !i.pinned; (i.id, i.pinned) };
        self.store.set_pinned(id, pinned);
        self.sort();
        self.refilter();
        if let Some(vi) = self.view.iter().position(|&i| self.items[i].id == id) { self.select(vi) }
        self.flash(if pinned { "pinned" } else { "unpinned" }, cx);
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(&ix) = self.view.get(self.sel) else { return };
        let i = self.items.remove(ix);
        self.store.delete(i.id);
        self.undo = Some(i);
        self.refilter();
        self.flash("deleted · ^z undo", cx);
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.undo.take() else { return };
        self.store.restore(&i);
        self.items.push(i);
        self.sort();
        self.refilter();
        self.flash("restored", cx);
    }

    fn set_cat(&mut self, c: usize) { self.cat = c % CATS.len(); self.sel = 0; self.refilter(); }

    // ─────────────── keyboard ───────────────

    fn on_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = e.keystroke.key.as_str();
        let m = e.keystroke.modifiers;

        if let Some(row) = self.settings {
            let n = options(row).len();
            match k {
                "up" => self.settings = Some(row.saturating_sub(1)),
                "down" => self.settings = Some((row + 1).min(SETTINGS.len() - 1)),
                "right" | "enter" | "space" => self.set_pref(row, self.prefs[row] + 1),
                "left" => self.set_pref(row, self.prefs[row] + n - 1),
                "escape" => self.settings = None,
                "," if m.control => self.settings = None,
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }

        match k {
            "down" => self.select(self.sel + 1),
            "up" => self.select(self.sel.saturating_sub(1)),
            "pagedown" => self.select(self.sel + 10),
            "pageup" => self.select(self.sel.saturating_sub(10)),
            "enter" => self.paste(m.shift, window, cx),
            "tab" => self.set_cat(self.cat + if m.shift { CATS.len() - 1 } else { 1 }),
            "right" => self.wide = true,
            "left" => self.wide = false,
            "delete" => self.delete(cx),
            "escape" => {
                if self.wide { self.wide = false }
                else if !self.query.is_empty() { self.query.clear(); self.refilter() }
                else { self.hide(window, cx) }
            }
            "backspace" => {
                if m.control { self.query.clear() } else { self.query.pop(); }
                self.refilter();
            }
            "p" if m.control => self.pin(cx),
            "z" if m.control => self.undo(cx),
            "," if m.control => self.settings = Some(0),
            _ => match e.keystroke.key_char.as_ref() {
                Some(ch) if !m.control && !m.alt => {
                    self.query.push_str(ch);
                    self.sel = 0;
                    self.refilter();
                }
                _ => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }
}

// ─────────────────────────────── helpers ───────────────────────────────

fn title(i: &Item) -> String {
    if i.kind == Kind::Image && i.thumb == i.text {
        return format!("image {}", i.meta);
    }
    match i.kind {
        Kind::Text | Kind::Code | Kind::Link => {
            i.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(300).collect()
        }
        _ => i.text.lines().map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p)).collect::<Vec<_>>().join("  "),
    }
}

/// Direction of the first strong character: Persian/Arabic/Hebrew → right-to-left.
fn is_rtl(s: &str) -> bool {
    for c in s.chars() {
        let u = c as u32;
        if (0x0590..=0x08FF).contains(&u) || (0xFB1D..=0xFDFF).contains(&u) || (0xFE70..=0xFEFF).contains(&u) {
            return true;
        }
        if c.is_alphabetic() { return false; }
    }
    false
}

/// RTL text gets a right-to-left embedding mark so mixed text and the "…" land on the correct side.
fn bidi(s: &str) -> (String, usize) {
    if is_rtl(s) { (format!("\u{202B}{s}"), '\u{202B}'.len_utf8()) } else { (s.to_string(), 0) }
}

/// Shorten `text` with "…" until it is at most `max_w` pixels wide (measured with real shaping,
/// so Persian and emoji are measured correctly).
fn fit(window: &Window, text: &str, size: f32, max_w: f32) -> String {
    let ts = window.text_system();
    let width = |s: &str| -> f32 {
        let run = TextRun { len: s.len(), font: gpui::font(FONT), color: gpui::black(), background_color: None, underline: None, strikethrough: None };
        ts.shape_line(s.to_string().into(), px(size), &[run], None).width.into()
    };
    if width(text) <= max_w { return text.to_string(); }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if width(&s) <= max_w { lo = mid } else { hi = mid - 1 }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "…"
}

/// Word-wrap to `cols` columns, at most `max` lines. Returns the lines and how many were left out.
fn wrap(text: &str, cols: usize, max: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut total = 0usize;
    for para in text.replace('\t', "    ").replace('\r', "").split('\n') {
        let chars: Vec<char> = para.chars().collect();
        let mut start = 0;
        loop {
            let end = (start + cols).min(chars.len());
            let cut = if end < chars.len() {
                chars[start..end].iter().rposition(|c| c.is_whitespace()).map(|p| start + p + 1).filter(|&p| p > start).unwrap_or(end)
            } else { end };
            total += 1;
            if out.len() < max { out.push(chars[start..cut].iter().collect::<String>()) }
            start = cut;
            if start >= chars.len() { break; }
        }
    }
    (out, total.saturating_sub(max))
}

/// Days since 1970-01-01 → (year, month, day).
fn ymd(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// History and settings live here; installs and updates never touch it.
fn data_dir() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("cobox")
}

// ─────────────────────────────── view ───────────────────────────────

impl Render for CoBox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.dark = match self.prefs[0] {
            1 => true,
            2 => false,
            _ => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
        };
        let (t, a, atext) = (self.t(), self.a(), self.atext());
        let key = |k: &'static str, what: &'static str| {
            div().flex().gap(px(5.)).child(div().text_color(rgb(t.fg)).child(k)).child(what)
        };
        let small = self.fs() - 1.5;

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative().size_full().flex().flex_col()
            .bg(rgb(t.bg)).text_color(rgb(t.fg)).font_family(FONT).text_size(px(self.fs()))
            .border_1().border_color(rgb(t.line))
            // ── prompt + categories ──
            .child(
                div().h(px(40.)).flex_none().flex().items_center().gap(px(8.)).px(px(14.))
                    .child(mark(self.fs() + 2., a[0]))
                    .child(div().font_weight(FontWeight::BOLD).child("cobox"))
                    .child(div().text_color(rgb(atext)).font_weight(FontWeight::BOLD).child("❯"))
                    .child(
                        div().flex_1().min_w_0().flex().items_center().overflow_hidden()
                            .child(bidi(&self.query).0)
                            .child(div().w(px(7.)).h(px(self.fs() + 2.)).bg(rgb(a[0])))
                            .when(self.query.is_empty(), |d| d.child(div().pl(px(8.)).text_color(rgb(t.faint)).child("search"))),
                    )
                    .children(CATS.iter().enumerate().map(|(i, name)| {
                        let on = i == self.cat;
                        div().id(("cat", i)).px(px(5.)).cursor_pointer().text_size(px(small))
                            .text_color(rgb(if on { atext } else { t.dim }))
                            .child(if on { format!("[{name}]") } else { name.to_string() })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| { this.set_cat(i); cx.notify(); }))
                    })),
            )
            // ── history ──
            .when(!self.wide, |d| d.child(
                uniform_list("rows", self.rows.len(), cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                    range.map(|r| this.row(r, window, cx)).collect::<Vec<_>>()
                }))
                .track_scroll(self.scroll.clone())
                .w_full()
                .flex_1()
                // min height 0 lets the list be shorter than its content, so the mouse wheel scrolls it.
                .min_h_0(),
            ))
            .child(self.preview(window, cx))
            // ── status bar ──
            .child(
                div().h(px(24.)).flex_none().flex().items_center().gap(px(14.)).px(px(14.))
                    .bg(rgb(t.panel)).text_size(px(small)).text_color(rgb(t.dim))
                    .child(key("↵", "paste")).child(key("tab", "type")).child(key("→", "view"))
                    .child(key("^p", "pin")).child(key("del", "remove"))
                    .child(div().flex_1())
                    .child(match &self.toast {
                        Some(m) => div().text_color(rgb(atext)).child(m.clone()),
                        None => div().child(format!("{} items", self.view.len())),
                    })
                    .child(div().id("gear").flex().gap(px(5.)).cursor_pointer()
                        .child(div().text_color(rgb(t.fg)).child("^,")).child("settings")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| { this.settings = Some(0); cx.notify(); }))),
            )
            .when(self.settings.is_some(), |d| d.child(self.settings_view(cx)))
    }
}

impl CoBox {
    fn row(&self, r: usize, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (t, a, atext) = (self.t(), self.a(), self.atext());
        let h = px(self.row_h());
        let small = self.fs() - 1.5;
        let vi = match self.rows[r] {
            Row::Head(ref label) => {
                return div().w_full().h(h).flex().items_center().gap(px(8.)).px(px(14.)).overflow_hidden().text_size(px(small)).text_color(rgb(t.faint))
                    .child("──")
                    .child(div().text_color(rgb(if label == "pinned" { atext } else { t.dim })).child(label.clone()))
                    .child(div().flex_1().h(px(1.)).bg(rgb(t.line)))
                    .into_any_element();
            }
            Row::Item(vi) => vi,
        };
        let i = &self.items[self.view[vi]];
        let on = vi == self.sel;
        // Cut to the space between the type and time columns, with "…" on the reading side.
        let tag_w = small * 2.6;
        let room = W - 28. - 2. - tag_w - small * 3.2 - 24.;
        let (text, shift) = bidi(&title(i));
        let text = fit(window, &text, self.fs(), room);
        let rtl = shift > 0;

        // Highlight the first search word.
        let word = self.query.split_whitespace().next().unwrap_or("").to_lowercase();
        let lower = text.to_lowercase();
        let hl = (!word.is_empty() && lower.len() == text.len())
            .then(|| lower[shift..].find(&word).map(|s| shift + s..shift + s + word.len()))
            .flatten()
            .filter(|r| r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        let styled = StyledText::new(text).with_highlights(hl.map(|r| (r, HighlightStyle {
            background_color: Some(rgb(a[0]).into()),
            color: Some(rgb(a[3]).into()),
            ..Default::default()
        })));
        let side = rgb(if on { atext } else { t.dim });

        // type ········· text (left or right aligned) ········· time
        div().id(("row", vi)).w_full().h(h).flex().items_center().gap(px(12.)).px(px(14.)).overflow_hidden().cursor_pointer()
            .border_l_2().border_color(if on { rgb(a[0]).into() } else { gpui::transparent_black() })
            .when(on, |d| d.bg(rgb(a[if self.dark { 4 } else { 5 }])))
            .child(div().w(px(tag_w)).flex_none().text_size(px(small)).text_color(side).child(i.kind.tag()))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().when(rtl, |d| d.text_right()).child(styled))
            .child(div().flex_none().text_size(px(small)).text_color(side).child(self.clock(i.ts)))
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                if this.settings.is_some() { return }
                this.sel = vi;
                if e.click_count() >= 2 { this.paste(false, window, cx) }
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                if this.settings.is_some() { return }
                this.sel = vi;
                this.pin(cx);
                cx.notify();
            }))
            .into_any_element()
    }

    fn preview(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.t();
        let big = self.wide;
        let fs = self.fs();
        let line_h = (fs * 1.55).round();
        let mut p = div().id("preview").flex_none().flex().flex_col().gap(px(6.)).px(px(14.)).py(px(8.)).overflow_hidden()
            .border_t_1().border_color(rgb(t.line)).cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| { this.wide = !this.wide; cx.notify(); }));
        p = if big { p.flex_1().min_h_0() } else { p.h(px(line_h * 3. + fs + 30.)) };
        let Some(i) = self.cur() else {
            return p.child(div().text_color(rgb(t.faint)).child("nothing yet · copy something"));
        };

        // How many lines/columns fit: monospace makes this exact for latin text.
        let cols = ((W - 30.) / (fs * 0.6)) as usize;
        let max = if big { ((H - 120.) / line_h) as usize } else { 3 };
        let is_text = matches!(i.kind, Kind::Text | Kind::Code | Kind::Link);
        // The compact preview skips blank lines so the three lines show actual content.
        let src_text = if big { i.text.clone() } else {
            i.text.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")
        };
        let (lines, more) = wrap(&src_text, if is_text { cols } else { cols - 18 }, max);

        let mut head = format!("{} · {} · {} · {} {}", i.kind.tag(), i.src, human(i.size), self.day_label(i.ts), self.clock(i.ts));
        if is_text { head += &format!(" · {} lines", i.text.lines().count().max(1)) }
        if more > 0 { head += &format!(" · +{more} more →") }
        let text_block = div().flex_1().min_w_0().flex().flex_col().overflow_hidden()
            .children(lines.into_iter().map(|l| {
                let (s, shift) = bidi(&l);
                let s = fit(window, &s, fs, W - 32.);
                div().flex_none().h(px(line_h)).line_height(px(line_h)).overflow_hidden().whitespace_nowrap()
                    .when(shift > 0, |d| d.text_right()).child(s)
            }));

        let body = div().flex_1().min_h_0().flex().gap(px(12.))
            .when(!i.thumb.is_empty(), |d| d.child(
                img(PathBuf::from(&i.thumb)).h(px(if big { 280. } else { line_h * 3. })).max_w(px(if big { 560. } else { 110. }))
                    .object_fit(ObjectFit::Contain).rounded(px(3.)),
            ))
            .child(text_block.when(!is_text, |d| d.text_color(rgb(t.dim))));
        p.child(div().flex_none().text_size(px(fs - 2.)).text_color(rgb(t.faint)).truncate().child(head)).child(body)
    }

    fn settings_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (t, a, atext) = (self.t(), self.a(), self.atext());
        let sel = self.settings.unwrap_or(0);
        let rows = SETTINGS.iter().enumerate().map(|(row, (label, _))| {
            let on = row == sel;
            let cur = self.prefs[row];
            let opts = options(row);
            let value = if row == 1 {
                // Color swatches.
                div().flex().gap(px(6.)).children(ACCENTS.iter().enumerate().map(|(ix, c)| {
                    div().id(("accent", ix)).size(px(16.)).cursor_pointer().rounded(px(3.)).bg(rgb(c[0])).border_2()
                        .border_color(rgb(if ix == cur { t.fg } else { t.panel }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| { this.set_pref(1, ix); cx.notify(); }))
                })).into_any_element()
            } else if opts.len() <= 3 {
                // Short lists: show every option, current in [brackets].
                div().flex().gap(px(4.)).children(opts.iter().enumerate().map(|(ix, o)| {
                    let picked = ix == cur;
                    div().id(("opt", row * 10 + ix)).cursor_pointer().text_color(rgb(if picked { atext } else { t.dim }))
                        .child(if picked { format!("[{o}]") } else { format!(" {o} ") })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| { this.set_pref(row, ix); cx.notify(); }))
                })).into_any_element()
            } else {
                // Long lists: ‹ current ›, click to cycle.
                div().id(("cycle", row)).flex().gap(px(6.)).cursor_pointer()
                    .child(div().text_color(rgb(t.faint)).child("‹"))
                    .child(div().text_color(rgb(atext)).child(opts[cur]))
                    .child(div().text_color(rgb(t.faint)).child("›"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| { this.set_pref(row, this.prefs[row] + 1); cx.notify(); }))
                    .into_any_element()
            };
            div().id(("set", row)).h(px(self.row_h())).flex().items_center().gap(px(10.)).px(px(10.)).cursor_pointer()
                .when(on, |d| d.bg(rgb(a[if self.dark { 4 } else { 5 }])))
                .child(div().w(px(10.)).text_color(rgb(a[0])).child(if on { "❯" } else { "" }))
                .child(div().w(px(160.)).text_color(rgb(if on { t.fg } else { t.dim })).child(*label))
                .child(value)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| { this.settings = Some(row); cx.notify(); }))
        });
        let panel = div().w(px(500.)).py(px(12.)).flex().flex_col().gap(px(2.)).bg(rgb(t.panel))
            .border_1().border_color(rgb(t.line)).rounded(px(6.))
            .child(div().px(px(14.)).pb(px(6.)).flex().gap(px(8.)).text_size(px(self.fs() - 1.5)).text_color(rgb(t.faint))
                .child("── settings").child(div().flex_1().h(px(1.)).mt(px(7.)).bg(rgb(t.line))))
            .children(rows)
            .child(div().px(px(14.)).pt(px(8.)).text_size(px(self.fs() - 1.5)).text_color(rgb(t.faint))
                .child("↑↓ choose · ←→ change · esc close"));
        // `occlude` keeps clicks from reaching the list underneath.
        div().id("settings-layer").absolute().inset_0().occlude().flex().items_center().justify_center()
            .bg(gpui::rgba(if self.dark { 0x0a0a0cb0 } else { 0x18181b40 }))
            .child(panel)
    }
}

/// Hide CoBox. Returns whether its window should also be closed (everywhere but Windows).
fn hide_window(hwnd: isize, cx: &mut App) -> bool {
    #[cfg(windows)]
    {
        let _ = cx;
        sys::hide(hwnd);
        false
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        cx.hide(); // macOS: hand focus back to the previous app
        true
    }
}

fn open_window(app: &Entity<CoBox>, cx: &mut App) -> Option<WindowHandle<CoBox>> {
    let bounds = Bounds::centered(None, size(px(W), px(H)), cx);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        // Linux pop-ups are notification windows, which don't get the keyboard.
        kind: if cfg!(target_os = "linux") { WindowKind::Normal } else { WindowKind::PopUp },
        focus: !cfg!(windows),
        show: !cfg!(windows),
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        app_id: Some("cobox".into()),
        window_decorations: Some(gpui::WindowDecorations::Client),
        ..Default::default()
    };
    let view = app.clone();
    let handle = cx.open_window(opts, move |window, cx| {
        view.update(cx, |this, cx| this.attach(window, cx));
        view
    }).ok()?;
    app.update(cx, |this, _| this.window = Some(handle));
    Some(handle)
}

fn toggle(app: &Entity<CoBox>, cx: &mut App) {
    let prev = sys::foreground();
    let existing = app.read(cx).window.and_then(|w| {
        w.update(cx, |this, window, cx| {
            // Windows keeps the window around hidden, so "open" means "active" there.
            if !cfg!(windows) || window.is_window_active() { this.hide(window, cx) } else { this.show(prev, window) }
        }).ok()
    });
    if existing.is_none() && let Some(w) = open_window(app, cx) {
        w.update(cx, |this, window, _| this.show(prev, window)).ok();
    }
}

fn main() {
    sys::single_instance();
    let dir = data_dir();
    std::fs::create_dir_all(dir.join("img")).ok();
    let store = Store::open(&dir).expect("cannot open database");

    Application::new().run(move |cx: &mut App| {
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        // Register the saved hotkey before the listener starts.
        sys::set_hotkey(store.get("s.hotkey").and_then(|v| v.parse().ok()).unwrap_or(0));
        sys::spawn(tx, dir.join("img"));

        let app = cx.new(|cx| CoBox::new(store, cx));
        // Windows: create the (hidden) window now so the first Alt+V is instant.
        if cfg!(windows) { open_window(&app, cx).expect("cannot open window"); }

        // Sleeps until the listener thread sends a clip or the hotkey fires.
        cx.spawn(async move |cx| {
            while let Some(ev) = rx.next().await {
                cx.update(|cx| match ev {
                    sys::Event::Toggle => toggle(&app, cx),
                    sys::Event::Clip(c) => app.update(cx, |this, cx| this.add(c, cx)),
                }).ok();
            }
        })
        .detach();
    });
}
