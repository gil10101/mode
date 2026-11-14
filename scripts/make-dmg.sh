#!/usr/bin/env bash
# Packages a built Mode.app into a .dmg with plain `hdiutil`, from a staging folder
# that holds the app and an Applications link, so the image needs no Finder
# scripting (Tauri's bundle_dmg.sh drives Finder through AppleScript to lay the
# window out, and on GitHub's macOS runners that path fails intermittently with
# `hdiutil: create failed - Resource busy`).
# Usage: scripts/make-dmg.sh <path/to/Mode.app> <out.dmg> [volume name]
set -euo pipefail
app="$1"; out="$2"; vol="${3:-Mode}"
[ -d "$app" ] || { echo "make-dmg: no app bundle at $app" >&2; exit 2; }
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
# A stale mount of a previous attempt keeps the volume name busy; detach it first.
for v in /Volumes/"$vol"*; do [ -d "$v" ] && hdiutil detach "$v" -force >/dev/null 2>&1 || true; done
mkdir -p "$(dirname "$out")"
# The runner's disk-image helper is occasionally busy for a few seconds after a
# previous image; that is the only thing retried here, five times at most.
attempt=1
until hdiutil create -volname "$vol" -srcfolder "$staging" -ov -format UDZO -fs HFS+ "$out" >/dev/null; do
  [ "$attempt" -lt 5 ] || { echo "make-dmg: hdiutil create failed after $attempt attempts" >&2; exit 1; }
  attempt=$((attempt + 1)); sleep 5
done
hdiutil verify "$out" >/dev/null
echo "make-dmg: wrote $out ($(du -h "$out" | cut -f1)) after $attempt attempt(s)"
