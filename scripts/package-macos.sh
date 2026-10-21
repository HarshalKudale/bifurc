#!/usr/bin/env bash
# Builds the production bundle on macOS and packages it as release/desktop/Bifurc-<version>-mac-<arch>.dmg.
# Usage: scripts/package-macos.sh [release-tag]     (run on a Mac; the dmg is built for this Mac's architecture)
# Needs: Xcode command line tools (xcode-select --install); Rust is installed with rustup if missing.
set -euo pipefail
cd "$(dirname "$0")/.."

# The build bakes BIFURC_GITHUB_CLIENT_ID / BIFURC_POSTHOG_* into the engine from the environment or .env.
[ -f .env ] || { echo ".env is missing at the repo root: the build would have no GitHub login or analytics keys" >&2; exit 1; }
set -a; . ./.env; set +a
: "${BIFURC_GITHUB_CLIENT_ID:?BIFURC_GITHUB_CLIENT_ID is not set in .env}"

[ "$(uname -s)" = Darwin ] || { echo "run this on macOS" >&2; exit 1; }
if ! command -v cargo >/dev/null 2>&1; then
  [ -f "$HOME/.cargo/env" ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "$HOME/.cargo/env"
fi

cargo xtask package ${1:+--release-tag "$1"}

echo
echo "Done. Output:"
ls -lh release/desktop/*.dmg
