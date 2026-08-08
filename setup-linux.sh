#!/usr/bin/env bash
set -euo pipefail

APP_NAME="Motional"
APP_ID="com.ejtbrown.motional"
BIN_NAME="motional-gui"
INSTALL_BINS=("motional-gui" "motional-cli" "motional-service")
ICON_NAME="${APP_ID}"
INSTALL_ROOT="${MOTIONAL_INSTALL_ROOT:-}"
INSTALL_BIN="/usr/bin/${BIN_NAME}"
DESKTOP_FILE="${INSTALL_ROOT}/usr/share/applications/${APP_ID}.desktop"
OLD_DESKTOP_FILE="${INSTALL_ROOT}/usr/share/applications/motional-gui.desktop"
ICON_FILE="${INSTALL_ROOT}/usr/share/icons/hicolor/512x512/apps/${ICON_NAME}.png"
OLD_ICON_FILE="${INSTALL_ROOT}/usr/share/icons/hicolor/512x512/apps/motional-gui.png"
PIXMAP_ICON_FILE="${INSTALL_ROOT}/usr/share/pixmaps/${ICON_NAME}.png"
OLD_PIXMAP_ICON_FILE="${INSTALL_ROOT}/usr/share/pixmaps/motional-gui.png"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLIENT_DIR="${REPO_ROOT}/apps/motional-lock"
SOURCE_BINARY_DIR="${CLIENT_DIR}/target/release"
PACKAGED_BINARY_DIR="${REPO_ROOT}"
SOURCE_ICON="${CLIENT_DIR}/assets/motional-icon.png"
PACKAGED_ICON="${REPO_ROOT}/motional-icon.png"

has_all_binaries() {
  local binary_dir="$1"
  local binary_name

  for binary_name in "${INSTALL_BINS[@]}"; do
    if [[ ! -x "${binary_dir}/${binary_name}" ]]; then
      return 1
    fi
  done

  return 0
}

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "setup-linux.sh can only be run on Linux." >&2
  exit 1
fi

if [[ "${EUID}" -ne 0 && -z "${INSTALL_ROOT}" ]]; then
  echo "This installer writes to /usr/bin and /usr/share/applications." >&2
  echo "Run it with sudo: sudo ./setup-linux.sh" >&2
  exit 1
fi

if has_all_binaries "${PACKAGED_BINARY_DIR}"; then
  BINARY_DIR="${PACKAGED_BINARY_DIR}"
elif [[ -f "${CLIENT_DIR}/Cargo.toml" ]]; then
  BINARY_DIR="${SOURCE_BINARY_DIR}"

  if ! has_all_binaries "${BINARY_DIR}"; then
    if ! command -v cargo >/dev/null 2>&1; then
      echo "cargo is required to build Motional binaries." >&2
      exit 1
    fi

    echo "Building Motional binaries..."
    cargo build --release --locked --bins --manifest-path "${CLIENT_DIR}/Cargo.toml"
  fi
else
  echo "Motional binaries were not found beside setup-linux.sh, and this is not a source checkout." >&2
  echo "Download and extract the complete Linux release archive before running the installer." >&2
  exit 1
fi

if ! has_all_binaries "${BINARY_DIR}"; then
  echo "One or more Motional binaries are missing from ${BINARY_DIR}." >&2
  exit 1
fi

if [[ -f "${PACKAGED_ICON}" ]]; then
  ICON_SOURCE="${PACKAGED_ICON}"
elif [[ -f "${SOURCE_ICON}" ]]; then
  ICON_SOURCE="${SOURCE_ICON}"
else
  echo "Motional icon was not found beside setup-linux.sh or in the source checkout." >&2
  exit 1
fi

for install_bin in "${INSTALL_BINS[@]}"; do
  echo "Installing ${install_bin} to ${INSTALL_ROOT}/usr/bin/${install_bin}..."
  install -D -m 0755 "${BINARY_DIR}/${install_bin}" "${INSTALL_ROOT}/usr/bin/${install_bin}"
done

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

if [[ -z "${INSTALL_ROOT}" ]] && command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

if [[ -z "${INSTALL_ROOT}" ]] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi

echo "Installed ${APP_NAME}."
