#!/bin/sh
# Installs the HyperLink Linux app for the current user (no root needed):
# the binary, its icons, and an app-menu entry.
#
#   linux/packaging/install-local.sh            # build (release) and install
#   linux/packaging/install-local.sh --no-build # install an existing build
#   linux/packaging/install-local.sh --uninstall
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN_DIR="$HOME/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_ID=com.hyperlink.Host

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN_DIR/hyperlink-linux" \
        "$DATA_DIR/applications/$APP_ID.desktop" \
        "$DATA_DIR/icons/hicolor/scalable/apps/$APP_ID.svg" \
        "$DATA_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
    echo "HyperLink removed. Your pairings in ~/.config/hyperlink were kept."
    exit 0
fi

if [ "${1:-}" != "--no-build" ]; then
    (cd "$ROOT" && cargo build --release -p hyperlink-linux --features video)
fi

install -Dm755 "$ROOT/target/release/hyperlink-linux" "$BIN_DIR/hyperlink-linux"
install -Dm644 "$ROOT/linux/data/icons/hicolor/scalable/apps/$APP_ID.svg" \
    "$DATA_DIR/icons/hicolor/scalable/apps/$APP_ID.svg"
install -Dm644 "$ROOT/linux/data/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" \
    "$DATA_DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"

# Same entry as the Flatpak's, but pointing at the installed binary.
mkdir -p "$DATA_DIR/applications"
sed "s|^Exec=.*|Exec=$BIN_DIR/hyperlink-linux|" \
    "$ROOT/linux/packaging/flatpak/$APP_ID.desktop" > "$DATA_DIR/applications/$APP_ID.desktop"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATA_DIR/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$DATA_DIR/icons/hicolor" || true

echo "HyperLink installed. Open it from your apps, or run: hyperlink-linux"
