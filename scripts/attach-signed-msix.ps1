<#
.SYNOPSIS
  Checks the Store-signed MSIX and attaches it to the GitHub release.

.DESCRIPTION
  Run this once Store certification is done and the update is live. Download the signed package by hand from
  store.rg-adguard.net (product id 9NCB4FDZZQPV, ring "Retail"), then pass it with -Path.
  (`winget download --source msstore` needs a Microsoft Entra ID sign-in with an administrator role, so it is not used.)

  Because rg-adguard is a third-party site, the package is checked before it is uploaded: a valid Microsoft signature,
  the right package name and version, and the publisher in packaging/windows/AppxManifest.xml.

.EXAMPLE
  scripts/attach-signed-msix.ps1 -Tag v0.6.5 -Path C:\Downloads\HarshalKudale.Bifurc_0.6.5.0_x64.msix
#>
param(
  [Parameter(Mandatory)][ValidatePattern('^v\d+\.\d+\.\d+$')][string]$Tag,
  [Parameter(Mandatory)][string]$Path,
  [string]$Repo = 'HarshalKudale/bifurc'
)
$ErrorActionPreference = 'Stop'
$version = $Tag.Substring(1)
$Path = (Resolve-Path $Path).Path

$signature = Get-AuthenticodeSignature $Path
if ($signature.Status -ne 'Valid') { throw "Signature is $($signature.Status), not Valid." }
# The Store signs with a per-app certificate (CN=<guid>) issued by a Microsoft Marketplace CA.
if ($signature.SignerCertificate.Issuer -notmatch 'CN=Microsoft Marketplace CA') {
  throw "Issued by $($signature.SignerCertificate.Issuer), not a Microsoft Marketplace CA."
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead($Path)
try {
  $entry = $zip.GetEntry('AppxManifest.xml')
  if (-not $entry) { throw 'The package has no AppxManifest.xml.' }
  $reader = New-Object System.IO.StreamReader($entry.Open())
  [xml]$manifest = $reader.ReadToEnd()
  $reader.Dispose()
} finally { $zip.Dispose() }
$identity = $manifest.Package.Identity
if ($identity.Name -ne 'HarshalKudale.Bifurc') { throw "Package name is $($identity.Name)." }
if ($identity.Version -ne "$version.0") { throw "Package version is $($identity.Version), expected $version.0." }

$repoRoot = Split-Path $PSScriptRoot -Parent
[xml]$ours = Get-Content (Join-Path $repoRoot 'packaging/windows/AppxManifest.xml')
# The Store re-signs the package under its own publisher id, so this only warns when the name differs.
if ($ours.Package.Identity.Publisher -ne $identity.Publisher) {
  Write-Warning "Publisher in the package ($($identity.Publisher)) differs from packaging/windows/AppxManifest.xml; the Store assigns its own."
}

$name = "Bifurc-$version-store-signed.msix"
$work = Join-Path ([IO.Path]::GetTempPath()) "bifurc-msix-$version"
New-Item -ItemType Directory -Force $work | Out-Null
Copy-Item $Path (Join-Path $work $name) -Force
$file = Join-Path $work $name
gh release upload $Tag $file --repo $Repo --clobber
if ($LASTEXITCODE) { throw 'gh release upload failed.' }

# Add its hash to SHA256SUMS.txt.
$hash = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
gh release download $Tag --repo $Repo --pattern SHA256SUMS.txt --dir $work --clobber
$sums = Join-Path $work 'SHA256SUMS.txt'
$lines = @(Get-Content $sums | Where-Object { $_ -notmatch [regex]::Escape($name) }) + "$hash  $name"
[IO.File]::WriteAllText($sums, ($lines -join "`n") + "`n")
gh release upload $Tag $sums --repo $Repo --clobber
if ($LASTEXITCODE) { throw 'gh release upload of SHA256SUMS.txt failed.' }
Write-Host "Attached $name to $Tag."
