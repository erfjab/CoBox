//! SQLite history. One table, one row per unique clip (deduped by content hash).

use rusqlite::{Connection, params};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind { Text, Code, Link, Image, Video, Pdf, Audio, File }

impl Kind {
    pub const ALL: [Kind; 8] = [Kind::Text, Kind::Code, Kind::Link, Kind::Image, Kind::Video, Kind::Pdf, Kind::Audio, Kind::File];
    pub fn tag(self) -> &'static str {
        ["txt", "code", "url", "img", "vid", "pdf", "aud", "file"][self as usize]
    }
    /// Broad category shown in the UI: 1 text, 2 link, 3 media, 4 file.
    pub fn group(self) -> usize {
        match self { Kind::Text | Kind::Code => 1, Kind::Link => 2, Kind::Image | Kind::Video | Kind::Audio => 3, Kind::Pdf | Kind::File => 4 }
    }
    fn from_i64(v: i64) -> Kind { Kind::ALL.get(v as usize).copied().unwrap_or(Kind::Text) }

    /// Classify a file by extension.
    pub fn of_path(p: &str) -> Kind {
        let ext = p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "heic" | "ico" => Kind::Image,
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" => Kind::Video,
            "mp3" | "wav" | "m4a" | "flac" | "ogg" | "aac" | "opus" => Kind::Audio,
            "pdf" => Kind::Pdf,
            _ => Kind::File,
        }
    }

    /// Classify copied text: link, code or plain text.
    pub fn of_text(t: &str) -> Kind {
        let s = t.trim();
        if !s.contains(char::is_whitespace) && (s.starts_with("http://") || s.starts_with("https://") || s.starts_with("www.")) {
            return Kind::Link;
        }
        let lines = s.lines().count();
        let codey = s.lines().filter(|l| {
            let l = l.trim_end();
            l.ends_with(';') || l.ends_with('{') || l.ends_with('}') || l.starts_with("    ") || l.starts_with('\t')
        }).count();
        if codey >= 2 || (lines == 1 && (s.starts_with("$ ") || s.starts_with("ssh ") || s.starts_with("git ") || s.starts_with("cargo "))) {
            Kind::Code
        } else {
            Kind::Text
        }
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: i64,
    pub kind: Kind,
    /// Text content, or newline-separated file paths, or the png path for bitmaps.
    pub text: String,
    /// Short info: "1920×1080", "3 files", "12 lines".
    pub meta: String,
    /// Thumbnail png (images, videos, pdfs) — empty if none.
    pub thumb: String,
    /// App the clip came from.
    pub src: String,
    pub ts: i64,
    pub pinned: bool,
    /// Bytes (text length, image file size, or total size of copied files).
    pub size: i64,
    /// Lowercased prefix used for search.
    pub lc: String,
}

/// A clip captured from the system clipboard, not stored yet.
pub struct NewClip {
    pub kind: Kind,
    pub text: String,
    pub meta: String,
    pub thumb: String,
    pub src: String,
    pub size: i64,
    pub hash: i64,
}

pub struct Store(Connection);

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// FNV-1a: tiny, fast, stable across builds (used to dedupe clips).
pub fn hash(bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
    }
    h as i64
}

impl Store {
    pub fn open(dir: &Path) -> rusqlite::Result<Self> {
        let c = Connection::open(dir.join("cobox.db"))?;
        c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS items(
               id INTEGER PRIMARY KEY, kind INTEGER, text TEXT, meta TEXT, thumb TEXT,
               src TEXT, ts INTEGER, pinned INTEGER DEFAULT 0, hash INTEGER UNIQUE);
             CREATE TABLE IF NOT EXISTS kv(k TEXT PRIMARY KEY, v TEXT);",
        )?;
        c.execute("ALTER TABLE items ADD COLUMN size INTEGER DEFAULT 0", []).ok(); // older dbs
        Ok(Store(c))
    }

    pub fn all(&self) -> Vec<Item> {
        let mut st = self.0
            .prepare("SELECT id,kind,text,meta,thumb,src,ts,pinned,size FROM items ORDER BY pinned DESC, ts DESC")
            .unwrap();
        st.query_map([], row).unwrap().flatten().collect()
    }

    /// Insert, or bump an existing identical clip to the top. Returns the stored row.
    pub fn upsert(&self, c: &NewClip) -> Option<Item> {
        self.0.query_row(
            "INSERT INTO items(kind,text,meta,thumb,src,ts,hash,size) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(hash) DO UPDATE SET ts=excluded.ts, src=excluded.src
             RETURNING id,kind,text,meta,thumb,src,ts,pinned,size",
            params![c.kind as i64, c.text, c.meta, c.thumb, c.src, now(), c.hash, c.size],
            row,
        ).ok()
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) {
        self.0.execute("UPDATE items SET pinned=?2 WHERE id=?1", params![id, pinned]).ok();
    }

    pub fn delete(&self, id: i64) {
        self.0.execute("DELETE FROM items WHERE id=?1", [id]).ok();
    }

    pub fn restore(&self, i: &Item) {
        self.0.execute(
            "INSERT OR IGNORE INTO items(id,kind,text,meta,thumb,src,ts,pinned,hash,size) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![i.id, i.kind as i64, i.text, i.meta, i.thumb, i.src, i.ts, i.pinned, hash(i.text.as_bytes()), i.size],
        ).ok();
    }

    /// Forget unpinned clips older than `days`. Returns the image files they owned.
    pub fn prune(&self, days: i64) -> Vec<String> {
        let cut = now() - days * 86400;
        let mut st = self.0.prepare("SELECT thumb FROM items WHERE pinned=0 AND ts<?1 AND thumb<>''").unwrap();
        let files = st.query_map([cut], |r| r.get(0)).unwrap().flatten().collect();
        self.0.execute("DELETE FROM items WHERE pinned=0 AND ts<?1", [cut]).ok();
        files
    }

    pub fn get(&self, k: &str) -> Option<String> {
        self.0.query_row("SELECT v FROM kv WHERE k=?1", [k], |r| r.get(0)).ok()
    }

    pub fn set(&self, k: &str, v: &str) {
        self.0.execute("INSERT OR REPLACE INTO kv(k,v) VALUES(?1,?2)", [k, v]).ok();
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    let text: String = r.get(2)?;
    let tlen = text.len() as i64;
    let lc = text.chars().take(4096).collect::<String>().to_lowercase();
    Ok(Item {
        id: r.get(0)?,
        kind: Kind::from_i64(r.get(1)?),
        text,
        meta: r.get(3)?,
        thumb: r.get(4)?,
        src: r.get(5)?,
        ts: r.get(6)?,
        pinned: r.get(7)?,
        size: r.get::<_, Option<i64>>(8)?.filter(|&s| s > 0).unwrap_or(tlen),
        lc,
    })
}

pub fn human(b: i64) -> String {
    match b {
        ..1024 => format!("{b} B"),
        1024..1048576 => format!("{:.1} KB", b as f64 / 1024.0),
        _ => format!("{:.1} MB", b as f64 / 1048576.0),
    }
}
