#!/bin/sh
# Installs or updates CoBox on macOS or Linux:
#   curl -fsSL https://raw.githubusercontent.com/erfjab/CoBox/master/install.sh | sh
# Only the program is replaced. History and settings (~/Library/Application Support/cobox
# on macOS, ~/.local/share/cobox on Linux) are never touched.
set -e

repo=erfjab/CoBox
case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  *) echo "CoBox: unsupported system $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "CoBox: unsupported CPU $(uname -m)" >&2; exit 1 ;;
esac
if [ "$os" = linux ] && [ "$arch" != x64 ]; then
  echo "CoBox: only 64-bit Intel/AMD Linux is supported for now" >&2; exit 1
fi

if [ "$os" = macos ]; then
  dest=/Applications
  [ -d "$HOME/Applications/CoBox.app" ] && dest="$HOME/Applications"
  [ -w "$dest" ] || { dest="$HOME/Applications"; mkdir -p "$dest"; }
  target="$dest/CoBox.app"
else
  bin="$HOME/.local/bin"
  share="${XDG_DATA_HOME:-$HOME/.local/share}"
  target="$bin/cobox"
fi
if [ -e "$target" ]; then update=1; echo "Updating CoBox..."; else update=; echo "Installing CoBox..."; fi

# Download and unpack first, so a failed download leaves the current install as it is.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "https://github.com/$repo/releases/latest/download/cobox-$os-$arch.tar.gz" | tar -xz -C "$tmp"

# Close the running copy (the database survives this) and wait for it to exit.
pkill -x cobox 2>/dev/null || true
i=0; while pgrep -x cobox >/dev/null 2>&1 && [ $i -lt 20 ]; do sleep 0.5; i=$((i + 1)); done

if [ "$os" = macos ]; then
  [ -d "$tmp/CoBox.app" ] || { echo "CoBox: the download did not contain CoBox.app" >&2; exit 1; }
  # Keep the old app until the new one is in place.
  rm -rf "$target.old"
  [ -n "$update" ] && mv "$target" "$target.old"
  if ! mv "$tmp/CoBox.app" "$target"; then
    [ -n "$update" ] && mv "$target.old" "$target"
    echo "CoBox: could not write $target" >&2; exit 1
  fi
  rm -rf "$target.old"
  xattr -dr com.apple.quarantine "$target" 2>/dev/null || true
  open "$target"
else
  [ -f "$tmp/cobox/cobox" ] || { echo "CoBox: the download did not contain cobox" >&2; exit 1; }
  mkdir -p "$bin" "$share/applications" "$share/icons/hicolor/256x256/apps"
  # Copy next to the target, then rename: the swap is atomic, so the old binary stays until it succeeds.
  install -m 755 "$tmp/cobox/cobox" "$target.new"
  mv -f "$target.new" "$target"
  cp "$tmp/cobox/cobox.png" "$share/icons/hicolor/256x256/apps/cobox.png"
  cat > "$share/applications/cobox.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=CoBox
Comment=Clipboard history
Exec=$target
Icon=cobox
Categories=Utility;
DESKTOP
  nohup "$target" >/dev/null 2>&1 &
fi

if [ -n "$update" ]; then echo "CoBox updated. Your history and settings are kept."
else echo "CoBox installed to $target"; fi
