#!/bin/sh
# Measures Bifurc's install size and idle memory on macOS and Linux.
#   scripts/bench/bench.sh <install folder> <ui executable>
# Prints JSON for the "results" of docs/assets/bench.json. Cold start to the first window needs a window
# system query and is only measured by bench.ps1 on Windows.
set -eu
dir="${1:?install folder}"; exe="${2:?ui executable}"
size_mb=$(du -sm "$dir" | cut -f1)
"$exe" >/dev/null 2>&1 &
pid=$!
sleep 30
# Resident set of the UI and everything it started, in MB (RSS counts shared pages; PSS is not on macOS).
rss_kb=$(ps -o rss= -p "$pid" $(pgrep -P "$pid" 2>/dev/null || true) | awk '{s+=$1} END{print s}')
kill "$pid" 2>/dev/null || true
printf '{ "name": "Bifurc", "installMb": %s, "idleMemoryMb": %s }\n' "$size_mb" "$((rss_kb / 1024))"
