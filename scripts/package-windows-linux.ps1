# Builds the production bundles for Windows (here) and Linux (in WSL) and packages them into release/desktop.
#   Windows: nsis, msix, zip, winget, scoop    Linux: appimage, deb, tarball, aur
# Usage: pwsh scripts/package-windows-linux.ps1 [-SkipLinux] [-SkipWindows] [-Distro Ubuntu] [-ReleaseTag v0.6.0]
# Needs on Windows: Rust, NSIS (makensis), Windows SDK (makeappx). In WSL: apt packages below (sudo asked once) and rustup (installed if missing).
param(
    [switch]$SkipWindows,
    [switch]$SkipLinux,
    [string]$Distro = "Ubuntu",
    [string]$ReleaseTag = ""
)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repo
# The build bakes BIFURC_GITHUB_CLIENT_ID / BIFURC_POSTHOG_* into the engine from the environment or .env.
$envFile = Join-Path $repo ".env"
if (-not (Test-Path $envFile)) { throw ".env is missing at the repo root: the build would have no GitHub login or analytics keys" }
foreach ($line in Get-Content $envFile) {
    if ($line -match '^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$' -and $line -notmatch '^\s*#') {
        [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2].Trim("'", '"'), "Process")
    }
}
if (-not $env:BIFURC_GITHUB_CLIENT_ID) { throw "BIFURC_GITHUB_CLIENT_ID is not set in .env" }
$tagArgs = if ($ReleaseTag) { "--release-tag $ReleaseTag" } else { "" }

if (-not $SkipWindows) {
    Write-Host "== Windows ==" -ForegroundColor Cyan
    cargo xtask package $(if ($ReleaseTag) { "--release-tag"; $ReleaseTag })
    if ($LASTEXITCODE) { throw "Windows packaging failed" }
}

if (-not $SkipLinux) {
    Write-Host "== Linux (WSL: $Distro) ==" -ForegroundColor Cyan
    $wslRepo = (wsl -d $Distro -- wslpath -a ($repo.Path -replace '\\', '/')).Trim()
    # The target dir lives on the WSL disk: building on /mnt/<drive> is very slow.
    $script = @"
set -euo pipefail
cd '$wslRepo'
set -a; . <(tr -d '\r' < .env); set +a
deps="build-essential clang cmake pkg-config libasound2-dev libfontconfig-dev libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev libvulkan1 dpkg-dev curl file libfuse2"
missing=`$(for p in `$deps; do dpkg -s "`$p" >/dev/null 2>&1 || echo "`$p"; done)
if [ -n "`$missing" ]; then sudo apt-get update && sudo apt-get install -y `$missing; fi
if ! command -v cargo >/dev/null 2>&1; then
  [ -f "`$HOME/.cargo/env" ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  . "`$HOME/.cargo/env"
fi
export CARGO_TARGET_DIR="`$HOME/.cache/bifurc-target"
cargo xtask package $tagArgs
"@ -replace "`r", ""
    wsl -d $Distro -- bash -c $script
    if ($LASTEXITCODE) { throw "Linux packaging failed" }
}

Write-Host "`nDone. Output:" -ForegroundColor Green
Get-ChildItem (Join-Path $repo "release/desktop") -File | Select-Object Name, @{n = "MB"; e = { [math]::Round($_.Length / 1MB, 1) } }
