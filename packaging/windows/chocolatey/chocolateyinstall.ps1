$ErrorActionPreference = 'Stop'

# The same installer the release page offers: per-user, no elevation, `bifurc` on the user's PATH.
$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  fileType       = 'exe'
  url64bit       = '{{URL}}'
  checksum64     = '{{SHA256}}'
  checksumType64 = 'sha256'
  silentArgs     = '/S'
  validExitCodes = @(0)
  softwareName   = 'Bifurc'
}

Install-ChocolateyPackage @packageArgs
