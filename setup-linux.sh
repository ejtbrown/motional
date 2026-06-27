#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Motional"
APP_ID="com.ejtbrown.motional"
BIN_NAME="motional-gui"
INSTALL_BIN="/usr/bin/${BIN_NAME}"
DESKTOP_FILE="/usr/share/applications/${APP_ID}.desktop"
OLD_DESKTOP_FILE="/usr/share/applications/motional-gui.desktop"
ICON_NAME="${APP_ID}"
ICON_FILE="/usr/share/icons/hicolor/512x512/apps/${ICON_NAME}.png"
OLD_ICON_FILE="/usr/share/icons/hicolor/512x512/apps/motional-gui.png"
PIXMAP_ICON_FILE="/usr/share/pixmaps/${ICON_NAME}.png"
OLD_PIXMAP_ICON_FILE="/usr/share/pixmaps/motional-gui.png"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_DIR="${REPO_ROOT}/apps/motional-lock"
SOURCE_BIN="${CLIENT_DIR}/target/release/${BIN_NAME}"
ICON_SOURCE="${CLIENT_DIR}/assets/motional-icon.png"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "setup-linux.sh can only be run on Linux." >&2
  exit 1
fi

if [[ "${EUID}" -ne 0 ]]; then
  echo "This installer writes to /usr/bin and /usr/share/applications." >&2
  echo "Run it with sudo: sudo ./setup-linux.sh" >&2
  exit 1
fi

if [[ ! -x "${SOURCE_BIN}" ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is required to build ${BIN_NAME}, and ${SOURCE_BIN} does not exist." >&2
    exit 1
  fi

  echo "Building ${BIN_NAME}..."
  cargo build --release --bin "${BIN_NAME}" --manifest-path "${CLIENT_DIR}/Cargo.toml"
fi

echo "Installing ${BIN_NAME} to ${INSTALL_BIN}..."
install -D -m 0755 "${SOURCE_BIN}" "${INSTALL_BIN}"

if [[ ! -f "${ICON_SOURCE}" ]]; then
  echo "Icon file not found: ${ICON_SOURCE}" >&2
  exit 1
fi

echo "Installing application icon..."
if command -v convert >/dev/null 2>&1; then
  install -d -m 0755 "$(dirname "${ICON_FILE}")"
  convert "${ICON_SOURCE}" -resize 512x512 "${ICON_FILE}"
else
  install -D -m 0644 "${ICON_SOURCE}" "${ICON_FILE}"
fi
install -D -m 0644 "${ICON_SOURCE}" "${PIXMAP_ICON_FILE}"

if [[ -f "${OLD_DESKTOP_FILE}" && "${OLD_DESKTOP_FILE}" != "${DESKTOP_FILE}" ]]; then
  echo "Removing old desktop launcher ${OLD_DESKTOP_FILE}..."
  rm -f "${OLD_DESKTOP_FILE}"
fi
for old_icon in "${OLD_ICON_FILE}" "${OLD_PIXMAP_ICON_FILE}"; do
  if [[ -f "${old_icon}" ]]; then
    echo "Removing old icon ${old_icon}..."
    rm -f "${old_icon}"
  fi
done

echo "Installing GNOME desktop launcher to ${DESKTOP_FILE}..."
install -D -m 0644 /dev/stdin "${DESKTOP_FILE}" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=${APP_NAME}
Comment=Motional automation client
Exec=${INSTALL_BIN}
Icon=${ICON_NAME}
Terminal=false
Categories=Utility;Settings;
StartupNotify=true
StartupWMClass=${APP_ID}
EOF

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi

echo "Installed ${APP_NAME}."
