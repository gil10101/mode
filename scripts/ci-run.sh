#!/usr/bin/env bash
# Runs a command in CI and, on failure, surfaces the relevant output lines as
# GitHub annotations so they are readable through the API without the log blob.
# Usage: scripts/ci-run.sh "<step title>" <command> [args...]
set -uo pipefail
title="$1"; shift
log="$(mktemp)"
"$@" 2>&1 | tee "$log.raw"
status=${PIPESTATUS[0]}
sed -E 's/\x1b\[[0-9;]*[A-Za-z]//g' "$log.raw" > "$log"
if [ "$status" -ne 0 ]; then
  { grep -aE "parity diff: " "$log" | sort -u | head -6;
    grep -aE "(^|\s)error(\[E[0-9]+\])?: |could not compile|test [A-Za-z0-9_:]+ \.\.\. FAILED|panicked at|assertion .*failed|^\s+left: |^\s+right: |^failures:$|^\s{4}[a-z_:]+$|lock file needs|No such file|error: |^smoke: |^release-check: |^check-workflows: " "$log" \
    | grep -avE "^\s*Compiling|^\s*Checking|parity diff: |^smoke: (ok|OK)|^smoke: (starting|IPC|database|overlay|relay|core|every|log file|ui/dist)" | tail -4; } \
    | while IFS= read -r line; do
        msg="${line//%/%25}"; msg="${msg//$'\r'/}"; msg="${msg//$'\n'/%0A}"
        printf '::error title=%s::%s\n' "$title" "${msg:0:900}"
      done
  { grep -aEn -B2 -A12 "^error(\[E[0-9]+\])?: |^---- .* stdout ----$" "$log" | head -60; echo "----- last 30 lines -----"; tail -n 30 "$log"; } | sed -e 's/%/%25/g' -e 's/\r//g' | tr '\n' '\001' | sed 's/\x01/%0A/g' \
    | { read -r t; printf '::notice title=%s (error context)::%s\n' "$title" "${t:0:3900}"; }
fi
exit "$status"
