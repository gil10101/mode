#!/bin/sh
# Installs Mode on this machine: the latest release built for it, or a build from this checkout.
#
#   curl -fsSL https://raw.githubusercontent.com/gil10101/mode/main/scripts/install.sh | sh
#   scripts/install.sh            # inside a checkout: the same
#   scripts/install.sh --source   # inside a checkout: build the app here and install that
#
# A release install picks the file for this OS and CPU (macOS .app.zip, Linux .deb or .AppImage;
# Windows the -setup.exe), checks it against the release's SHA256SUMS.txt and installs it:
# Mode.app into /Applications, the .deb with dpkg, the AppImage into ~/.local/bin. The files
# come through `gh` when it is signed in (it can read a draft release), otherwise from the
# public release page.
#
# A source build needs Rust stable, Node 22 and the platform webview toolchain.
# It builds the hook relay and the app in release mode and installs the bundle the same way.
#
# The builds are unsigned (the certificates are parked until v1.0): macOS asks for a confirmation
# in System Settings > Privacy & Security the first time Mode opens, and Windows shows a
# SmartScreen prompt. Hooks for Claude Code are installed from inside the app, with a diff preview.
set -eu

REPO="gil10101/mode"
APP="Mode"

say() { printf '%s\n' "install: $*"; }
die() { printf '%s\n' "install: $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# ---- what machine this is ------------------------------------------------------------------
os=$(uname -s)
cpu=$(uname -m)
case "$os" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) die "unsupported OS: $os" ;;
esac
case "$cpu" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64) arch=x64 ;;
  *) die "unsupported CPU: $cpu" ;;
esac

# The checkout this script lives in, when it is run from one.
here=$(cd "$(dirname "$0")" 2>/dev/null && pwd) || here=""
root=""
if [ -n "$here" ] && [ -f "$here/../crates/mode-app/tauri.conf.json" ]; then
  root=$(cd "$here/.." && pwd)
fi

mode=release
for a in "$@"; do
  case "$a" in
    --source) mode=source ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) die "unknown argument: $a (try --source)" ;;
  esac
done
if [ "$mode" = source ] && [ -z "$root" ]; then
  die "--source needs a checkout: git clone https://github.com/$REPO && mode/scripts/install.sh --source"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

# ---- put a bundle in place ----------------------------------------------------------------
install_mac_app() { # <Mode.app or .app.zip>
  src="$1"
  if [ "${src##*.}" = zip ]; then
    ditto -xk "$src" "$tmp/app"
    src="$tmp/app/$APP.app"
  fi
  [ -d "$src" ] || die "no $APP.app in the bundle"
  if [ -d "/Applications/$APP.app" ]; then rm -rf "/Applications/$APP.app"; fi
  ditto "$src" "/Applications/$APP.app"
  say "installed /Applications/$APP.app"
  say "first open: Finder > Applications > $APP, then System Settings > Privacy & Security > Open Anyway (the build is unsigned)"
}

install_linux() { # <.deb or .AppImage>
  src="$1"
  case "$src" in
    *.deb)
      if [ "$(id -u)" = 0 ]; then dpkg -i "$src"; else say "installing the .deb with sudo dpkg -i"; sudo dpkg -i "$src"; fi
      say "installed: run 'mode' or open $APP from the app menu" ;;
    *.AppImage)
      mkdir -p "$HOME/.local/bin"
      cp "$src" "$HOME/.local/bin/mode"
      chmod +x "$HOME/.local/bin/mode"
      say "installed $HOME/.local/bin/mode (add ~/.local/bin to PATH if it is not there)" ;;
    *) die "unknown bundle: $src" ;;
  esac
}

install_windows() { # <-setup.exe>
  say "starting the installer: $1"
  cmd.exe /c start "" "$(cygpath -w "$1" 2>/dev/null || printf '%s' "$1")"
}

# ---- a release --------------------------------------------------------------------------------
release_asset_pattern() {
  case "$os-$arch" in
    macos-arm64) printf '%s' "${APP}_*_aarch64.app.zip" ;;
    macos-x64) printf '%s' "${APP}_*_x64.app.zip" ;;
    linux-x64) if have dpkg; then printf '%s' "${APP}_*_amd64.deb"; else printf '%s' "${APP}_*_amd64.AppImage"; fi ;;
    linux-arm64) die "no Linux arm64 release yet: build it with --source" ;;
    windows-x64) printf '%s' "${APP}_*_x64-setup.exe" ;;
    windows-arm64) die "no Windows arm64 release yet: build it with --source" ;;
  esac
}

verify() { # <dir with the asset and SHA256SUMS.txt> <asset>
  if [ ! -f "$1/SHA256SUMS.txt" ]; then say "no SHA256SUMS.txt in the release; skipping the checksum"; return 0; fi
  line=$(grep " $2\$" "$1/SHA256SUMS.txt" || grep " \*$2\$" "$1/SHA256SUMS.txt" || true)
  [ -n "$line" ] || die "$2 is not listed in SHA256SUMS.txt"
  if have sha256sum; then (cd "$1" && printf '%s\n' "$line" | sha256sum -c --quiet -) || die "checksum mismatch for $2"
  elif have shasum; then (cd "$1" && printf '%s\n' "$line" | shasum -a 256 -c --quiet -) || die "checksum mismatch for $2"
  else say "no sha256sum or shasum; skipping the checksum"; fi
  say "checksum ok"
}

from_release() {
  pattern=$(release_asset_pattern)
  got=""
  if have gh && gh auth status >/dev/null 2>&1; then
    # Newest release, drafts included (the owner's draft is the only one while Mode is in alpha).
    tag=$(gh release list --repo "$REPO" --limit 1 --json tagName -q '.[0].tagName' 2>/dev/null || true)
    if [ -n "$tag" ]; then
      say "release $tag through gh"
      gh release download "$tag" --repo "$REPO" --dir "$tmp/dl" --pattern "$pattern" --pattern SHA256SUMS.txt --clobber && got=1
    fi
  fi
  if [ -z "$got" ]; then
    # The public release page: the file name needs the version, which the latest tag gives.
    tag=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null | sed 's#.*/tag/##') || tag=""
    [ -n "$tag" ] && [ "$tag" != latest ] || return 1
    version=${tag#v}
    name=$(printf '%s' "$pattern" | sed "s/\*/$version/")
    say "release $tag from github.com"
    mkdir -p "$tmp/dl"
    curl -fsSL -o "$tmp/dl/$name" "https://github.com/$REPO/releases/download/$tag/$name" || return 1
    curl -fsSL -o "$tmp/dl/SHA256SUMS.txt" "https://github.com/$REPO/releases/download/$tag/SHA256SUMS.txt" || true
  fi
  asset=$(ls "$tmp/dl" | grep -v '^SHA256SUMS.txt$' | head -n 1)
  [ -n "$asset" ] || return 1
  verify "$tmp/dl" "$asset"
  case "$os" in
    macos) install_mac_app "$tmp/dl/$asset" ;;
    linux) install_linux "$tmp/dl/$asset" ;;
    windows) install_windows "$tmp/dl/$asset" ;;
  esac
}

# ---- a build from this checkout ----------------------------------------------------------------
from_source() {
  have cargo || die "Rust is needed: https://rustup.rs"
  have node || die "Node 22 is needed: https://nodejs.org"
  major=$(node -p 'process.versions.node.split(".")[0]')
  [ "$major" -ge 22 ] || die "Node 22 or newer is needed (found $(node -v))"
  case "$os" in macos) bundles=app ;; linux) if have dpkg; then bundles=deb; else bundles=appimage; fi ;; windows) bundles=nsis ;; esac
  say "building $APP from $root ($bundles)"
  (cd "$root/ui" && npm ci --no-audit --no-fund && npm run tauri -- build --bundles "$bundles")
  out="$root/target/release/bundle"
  case "$os" in
    macos) install_mac_app "$out/macos/$APP.app" ;;
    linux) f=$(ls "$out/deb/"*.deb 2>/dev/null | head -n 1 || true); [ -n "$f" ] || f=$(ls "$out/appimage/"*.AppImage | head -n 1); install_linux "$f" ;;
    windows) install_windows "$(ls "$out/nsis/"*-setup.exe | head -n 1)" ;;
  esac
}

if [ "$mode" = source ]; then
  from_source
elif from_release; then
  :
elif [ -n "$root" ]; then
  say "no release for this machine; building from the checkout instead"
  from_source
else
  die "no release found for $os $arch. Sign in with 'gh auth login' (private repo), or clone and run scripts/install.sh --source"
fi
