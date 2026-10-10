# gpui 0.2.2 — CoBox patches

Unmodified gpui 0.2.2 from crates.io except:

1. `src/platform/windows/direct_write.rs` (`DrawGlyphRun`): right-to-left runs
   (Persian, Arabic, Hebrew) are placed from the run's right edge, using the run
   origin DirectWrite reports. Upstream drew them left-to-right, so the letters
   came out in reverse order. Diacritic vertical offsets are applied too.
2. `build.rs`: finds `fxc.exe` in the newest installed Windows SDK instead of
   only `10.0.26100.0`.
3. `Cargo.toml`: examples/tests removed.
4. `src/platform/linux/{x11,wayland}/client.rs`: closing the last window no longer
   quits the app. On Linux CoBox closes its window to hide it and opens a new one
   on the hotkey, so it must keep running with no windows.
