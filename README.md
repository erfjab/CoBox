# CoBox

A small, keyboard-first clipboard history for Windows.

- Saves everything you copy: text, code, links, images, videos, PDFs, audio and files.
- Keeps your history until you delete it.
- Runs in the background and starts with Windows.
- Uses almost no CPU while idle.

## Requirements

- Windows 10 or 11
- [Rust](https://rustup.rs) with the MSVC toolchain
- Windows 10/11 SDK (install it with the Visual Studio Build Tools)

## Install

Open PowerShell in the project folder and run:

```powershell
cargo build --profile dist
taskkill /im cobox.exe /f 2>$null
mkdir "$env:LOCALAPPDATA\Programs\CoBox" -Force
copy target\dist\cobox.exe "$env:LOCALAPPDATA\Programs\CoBox\"
& "$env:LOCALAPPDATA\Programs\CoBox\cobox.exe"
```

The first build takes a few minutes.

Always start CoBox from the install folder, not from `target`. On launch it adds
itself to Windows startup using its own path.

## Update

Pull or edit the code, then run the same commands as in Install.
Your history and settings are kept.

## Uninstall

1. Open CoBox, press `Ctrl+,` and set **start with windows** to `off`.
2. Run `taskkill /im cobox.exe /f`.
3. Delete `%LOCALAPPDATA%\Programs\CoBox`.
4. To also delete your history and settings, delete `%APPDATA%\cobox`.

## Use

Press `Alt+V` to open CoBox. Start typing to search.

| Key | Action |
|---|---|
| type | search |
| `Up` / `Down` | move |
| `Enter` | paste into the previous window |
| `Shift+Enter` | paste as plain text |
| `Tab` | next type: all, text, link, media, file |
| `Right` / `Left` | big preview / back |
| `Ctrl+P` | pin or unpin |
| `Del` | remove |
| `Ctrl+Z` | undo remove |
| `Ctrl+,` | settings |
| `Esc` | clear search, then hide |

With the mouse: click to select, double-click to paste, right-click to pin,
scroll to move through the list.

`Esc`, `Alt+F4` and clicking outside only hide CoBox. It keeps running.

## Settings

Press `Ctrl+,`. Use `Up` / `Down` to choose a setting and `Left` / `Right` to change it.

| Setting | Options |
|---|---|
| theme | auto, dark, light |
| primary | 5 colors |
| open with | `alt+v`, `ctrl+shift+v`, `ctrl+alt+v`, `alt+c`, ``ctrl+` `` |
| start with windows | on, off |
| keep history | forever, 1 year, 3 months, 1 month, 1 week (pinned items are always kept) |
| skip passwords | on, off (ignore copies that password managers mark as private) |
| text size | 12, 13, 14 |
| dim screen | off, light, medium, dark |

## Develop

```powershell
cargo run          # fast debug build
.\run.bat          # stop the running copy, build, and start the new one
```

Only one CoBox runs at a time. A second copy exits right away.

| File | Purpose |
|---|---|
| `src/main.rs` | user interface: search, list, preview, settings |
| `src/store.rs` | history database |
| `src/win.rs` | Windows parts: clipboard, hotkey, paste, thumbnails, startup |
| `src/stub.rs` | empty stand-in so the UI builds on other systems |
| `vendor/gpui` | GPUI 0.2.2 with a right-to-left text fix (see `PATCHES.md`) |

Data is stored in `%APPDATA%\cobox`.
