#!/usr/bin/env bash
# Release checklist: the rows of docs/08-architecture.md §13.5 a script can
# check, plus hygiene. Exits 1 when any check finds something; prints
# `path:line: text` per finding (and GitHub annotations under Actions).
#
#   currency       `$` amounts, USD / EUR / GBP, € £ ¥, price / pricing /
#                  cost(s) in UI strings: ui/src (comments, tests, the
#                  dev gallery and ui/src/app/stats, the Stats view's spend
#                  row, ADR-0051, left out), string literals of
#                  crates/mode-app/src outside tests, crates/mode-core/src/**/*.txt
#                  prompts, the bundle descriptions in tauri.conf.json.
#                  Identifiers such as `cost_usd` / `totalCostUSD` never match
#                  (word boundaries); docs/ is not scanned.
#   branding       "Powered by", "Claude Code Agent" in the same scope plus
#                  README.md and ui/*.html.
#   key caps       `.m-key`, `<Key`, `<MenuItem shortcut=` outside
#                  ui/src/app/shortcuts/** (the kit that defines them, tests
#                  and the dev gallery left out; ADR-0032).
#   console.log    anywhere in ui/src (comments left out).
#   debug macros   `dbg!`, `todo!()`, `unimplemented!()` anywhere in crates/.
#   design         ui/scripts/check-design.mjs: color, type, radius and
#                  markup outside the kit in ui/src/app (docs/10-ui-ux.md §7).
#   site           ui/scripts/check-site.mjs: the landing page (site/) keeps the
#                  kit tokens, every reference resolves, the copy rules hold.
#   versions       tauri.conf.json = crates/mode-app/Cargo.toml (or the
#                  workspace version it inherits) = ui/package.json =
#                  ui/package-lock.json = Cargo.lock's mode-app and mode-hook
#                  entries (the relay reports the app's version), and
#                  `--tag vX.Y.Z` when given.
#
# Usage: scripts/release-check.sh [--tag v0.1.0]
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
command -v python3 >/dev/null || { echo "release-check: python3 is required"; exit 2; }
exec python3 -I - "$ROOT" "$@" <<'PY'
import json
import os
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(sys.argv[1])
ARGS = sys.argv[2:]
TAG = None
i = 0
while i < len(ARGS):
    if ARGS[i] == "--tag" and i + 1 < len(ARGS):
        TAG = ARGS[i + 1]
        i += 2
    elif ARGS[i].startswith("--tag="):
        TAG = ARGS[i][len("--tag="):]
        i += 1
    else:
        print(f"release-check: unknown argument {ARGS[i]}")
        sys.exit(2)

ANNOTATE = os.environ.get("GITHUB_ACTIONS") == "true"
failures = {}


def rel(p):
    return p.relative_to(ROOT).as_posix()


def find(check, path, line, text, why=""):
    failures.setdefault(check, []).append((rel(path), line, text.strip()[:160], why))


# ----------------------------------------------------------------- lexing


def lex(text, lang):
    """Returns (code, bare, strings): `code` has comments blanked, `bare`
    also has string contents blanked (both keep every newline, so offsets
    map to the same lines), `strings` lists (offset, content) of string
    literals. Languages: ts (also tsx / js), css, rs."""
    code = list(text)
    bare = list(text)
    strings = []
    n = len(text)
    i = 0

    def blank(buf, a, b):
        for k in range(a, b):
            if buf[k] != "\n":
                buf[k] = " "

    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//" and lang != "css":
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(code, i, j)
            blank(bare, i, j)
            i = j
        elif two == "/*":
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            blank(code, i, j)
            blank(bare, i, j)
            i = j
        elif lang == "rs" and c == "r" and re.match(r'r#*"', text[i:]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = re.match(r'r(#*)"', text[i:]).group(1)
            start = i + 2 + len(hashes)
            j = text.find('"' + hashes, start)
            j = n if j < 0 else j
            strings.append((start, text[start:j]))
            blank(bare, start, j)
            i = j + 1 + len(hashes)
        elif lang == "rs" and c == "'":
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:])
            i += len(m.group(0)) if m else 1  # a char literal, or a lifetime
        elif c in "\"'`" and not (lang == "rs" and c != '"'):
            j = i + 1
            while j < n and text[j] != c:
                if text[j] == "\\":
                    j += 1
                elif text[j] == "\n" and c != "`" and lang != "rs":
                    break  # unterminated: not a string after all
                j += 1
            strings.append((i + 1, text[i + 1:j]))
            blank(bare, i + 1, j)
            i = j + 1
        else:
            i += 1
    return "".join(code), "".join(bare), strings


def line_of(text, offset):
    return text.count("\n", 0, offset) + 1


def line_text(text, offset):
    a = text.rfind("\n", 0, offset) + 1
    b = text.find("\n", offset)
    return text[a:] if b < 0 else text[a:b]


def files(base, suffixes):
    d = ROOT / base
    if not d.exists():
        return []
    return sorted(p for p in d.rglob("*") if p.is_file() and p.suffix in suffixes and "node_modules" not in p.parts)


def lang_of(p):
    return {".rs": "rs", ".css": "css"}.get(p.suffix, "ts")


def is_test(path):
    r = rel(path)
    return re.search(r"\.test\.[jt]sx?$|/test/|/e2e/|wiring_tests\.rs$", r) is not None


# ---------------------------------------------------------------- patterns

CURRENCY = [
    (re.compile(r"(?<![\w\\$])\$\s?\d"), "a dollar amount"),
    (re.compile(r"\b(?:USD|EUR|GBP)\b"), "a currency code"),
    (re.compile(r"[€£¥]"), "a currency sign"),
    (re.compile(r"\b(?:price[sd]?|pricing|costs?|costly|costing)\b", re.I), "a price word"),
]
BRANDING = [
    (re.compile(r"powered\s+by", re.I), '"Powered by"'),
    (re.compile(r"claude\s+code\s+agent", re.I), '"Claude Code Agent"'),
]
UI_STRING_PATTERNS = CURRENCY + BRANDING


def check_text_block(check, path, full, segment, seg_offset, patterns, skip_line=None):
    for rx, why in patterns:
        for m in rx.finditer(segment):
            off = seg_offset + m.start()
            line = line_text(full, off)
            if skip_line and skip_line(line, why):
                continue
            find(check, path, line_of(full, off), line, why)


def replace_call(line, why):
    # `.replace(/x/, '$1')`: a capture reference, not an amount.
    return why == "a dollar amount" and re.search(r"\breplace(All)?\(", line) is not None


# ------------------------------------------------------- 1. UI strings: ui/src

ui_files = [p for p in files("ui/src", {".ts", ".tsx", ".js", ".jsx", ".css"})]
for p in ui_files:
    if is_test(p) or rel(p).startswith("ui/src/dev/"):
        continue
    text = p.read_text(encoding="utf-8")
    code, _bare, _strings = lex(text, lang_of(p))
    # The Stats view estimates spend at list price (ADR-0051): currency is allowed there, branding is not.
    patterns = BRANDING if rel(p).startswith("ui/src/app/stats/") else UI_STRING_PATTERNS
    check_text_block("currency and branding (ui/src)", p, text, code, 0, patterns, replace_call)

for p in sorted((ROOT / "ui").glob("*.html")):
    text = p.read_text(encoding="utf-8")
    check_text_block("currency and branding (ui/*.html)", p, text, text, 0, UI_STRING_PATTERNS)

# ---------------------------------- 2. user-facing strings: crates/mode-app/src

for p in files("crates/mode-app/src", {".rs"}):
    if is_test(p):
        continue
    text = p.read_text(encoding="utf-8")
    cut = re.search(r"^#\[cfg\(test\)\]\s*\n\s*mod \w+", text, re.M)
    body = text[: cut.start()] if cut else text
    _code, _bare, strings = lex(body, "rs")
    for off, s in strings:
        check_text_block("currency and branding (mode-app strings)", p, text, s, off, UI_STRING_PATTERNS)

# ------------------------------------------- 3. prompts: crates/mode-core/src/**/*.txt

for p in files("crates/mode-core/src", {".txt"}):
    text = p.read_text(encoding="utf-8")
    check_text_block("currency and branding (prompts)", p, text, text, 0, UI_STRING_PATTERNS)

# ---------------------------------------------------- 4. store text and README

conf_path = ROOT / "crates/mode-app/tauri.conf.json"
conf_text = conf_path.read_text(encoding="utf-8")
conf = json.loads(conf_text)
for key in ("shortDescription", "longDescription", "copyright"):
    value = conf.get("bundle", {}).get(key)
    if isinstance(value, str):
        off = conf_text.find(json.dumps(value, ensure_ascii=False)[1:-1])
        check_text_block("currency and branding (bundle text)", conf_path, conf_text, value, max(off, 0), UI_STRING_PATTERNS)
for key in ("productName",):
    value = conf.get(key, "")
    for rx, why in BRANDING:
        if rx.search(value):
            find("currency and branding (bundle text)", conf_path, 1, value, why)

readme = ROOT / "README.md"
if readme.exists():
    text = readme.read_text(encoding="utf-8")
    check_text_block("branding (README.md)", readme, text, text, 0, BRANDING)

# ------------------------------------- 5. key caps outside the Shortcuts view

KEYCAP = [
    (re.compile(r"(?<![\w-])\.?m-key\b(?!-)"), "a .m-key key cap"),
    (re.compile(r"<Key[\s/>]"), "a <Key> cap"),
    (re.compile(r"<MenuItem\b[^>]*?\bshortcut="), "a menu shortcut (renders .m-key)"),
]
# ADR-0062: the onboarding Dictation step prints the hold key in caps ("Hold ⌃ ⌥ D,
# speak, let go."), the one surface outside Shortcuts allowed a <Key>; its .m-key
# and menu shortcuts are still checked.
KEYCAP_ALLOWED = {"ui/src/app/onboarding/Onboarding.tsx": {"a <Key> cap"}}
for p in ui_files:
    r = rel(p)
    if is_test(p) or r.startswith(("ui/src/app/shortcuts/", "ui/src/kit/", "ui/src/dev/")):
        continue
    text = p.read_text(encoding="utf-8")
    code, _bare, _strings = lex(text, lang_of(p))
    rules = [rule for rule in KEYCAP if rule[1] not in KEYCAP_ALLOWED.get(r, ())]
    check_text_block("key caps outside ui/src/app/shortcuts", p, text, code, 0, rules)

# ------------------------------------------------------------ 6. console.log

for p in ui_files:
    text = p.read_text(encoding="utf-8")
    _code, bare, _strings = lex(text, lang_of(p))
    check_text_block("console.log in ui/src", p, text, bare, 0, [(re.compile(r"\bconsole\.log\s*\("), "console.log")])

# --------------------------------------------- 7. dbg! / todo!() / unimplemented!()

MACROS = [(re.compile(r"(?<![\w:])(?:dbg|todo|unimplemented)!\s*[\(\[\{]"), "a debug / placeholder macro")]
for p in files("crates", {".rs"}):
    if "target" in p.relative_to(ROOT).parts:
        continue
    text = p.read_text(encoding="utf-8")
    _code, bare, _strings = lex(text, "rs")
    check_text_block("dbg! / todo!() / unimplemented!() in crates", p, text, bare, 0, MACROS)

# -------------------------------------------------------------- 8. versions

versions = {}
versions["crates/mode-app/tauri.conf.json"] = conf.get("version")
cargo_ws = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
ws_version = cargo_ws.get("workspace", {}).get("package", {}).get("version")


def crate_version(name):
    pkg = tomllib.loads((ROOT / f"crates/{name}/Cargo.toml").read_text(encoding="utf-8"))["package"]
    v = pkg.get("version")
    return ws_version if isinstance(v, dict) and v.get("workspace") else v


versions["crates/mode-app/Cargo.toml" + (" (workspace)" if crate_version("mode-app") == ws_version else "")] = crate_version("mode-app")
pkg_json = json.loads((ROOT / "ui/package.json").read_text(encoding="utf-8"))
versions["ui/package.json"] = pkg_json.get("version")
lock_json_path = ROOT / "ui/package-lock.json"
if lock_json_path.exists():
    lock_json = json.loads(lock_json_path.read_text(encoding="utf-8"))
    versions["ui/package-lock.json"] = lock_json.get("version")
    versions['ui/package-lock.json packages[""]'] = lock_json.get("packages", {}).get("", {}).get("version")
lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
for name in ("mode-app", "mode-hook"):
    entry = next((p for p in lock.get("package", []) if p.get("name") == name and "source" not in p), None)
    versions[f"Cargo.lock {name}"] = entry and entry.get("version")
versions["crates/mode-hook/Cargo.toml (the relay's --version)"] = crate_version("mode-hook")
if TAG is not None:
    versions[f"tag {TAG}"] = TAG[1:] if TAG.startswith("v") else TAG

expected = versions["crates/mode-app/tauri.conf.json"]
for where, v in versions.items():
    if v != expected:
        where_path = ROOT / where.split(" ")[0]
        find("versions agree", where_path if where_path.is_file() else conf_path, 1, f"{where} = {v!r}, tauri.conf.json = {expected!r}", "version mismatch")
if expected is None or not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", str(expected)):
    find("versions agree", conf_path, 1, f"version {expected!r} is not semver", "not semver")

# ---------------------------------------------------------------- report

CHECKS = [
    "currency and branding (ui/src)",
    "currency and branding (ui/*.html)",
    "currency and branding (mode-app strings)",
    "currency and branding (prompts)",
    "currency and branding (bundle text)",
    "branding (README.md)",
    "key caps outside ui/src/app/shortcuts",
    "console.log in ui/src",
    "dbg! / todo!() / unimplemented!() in crates",
    "versions agree",
]
for check in CHECKS:
    found = failures.get(check, [])
    print(f"{'FAIL' if found else 'ok  '}  {check}" + (f" ({len(found)})" if found else ""))
    for path, line, text, why in found:
        print(f"        {path}:{line}: {why}: {text}")
        if ANNOTATE:
            msg = f"{why}: {text}".replace("%", "%25").replace("\n", "%0A")
            print(f"::error file={path},line={line},title=release-check {check}::{msg}")
print(f"release-check: version {expected}" + ("" if TAG is None else f", tag {TAG}"))
if failures:
    print(f"release-check: {sum(len(v) for v in failures.values())} finding(s)")
    sys.exit(1)
print("release-check: ok")
PY
# The design gate (docs/10-ui-ux.md §7): the packages stay inside the kit. The public copy of the
# repository (scripts/export-public.sh) carries neither the specs nor these two checks; they ran
# here before the copy was written.
if [ -f "$ROOT/ui/scripts/check-design.mjs" ]; then
  node "$ROOT/ui/scripts/check-design.mjs"
  node "$ROOT/ui/scripts/check-site.mjs"
fi
