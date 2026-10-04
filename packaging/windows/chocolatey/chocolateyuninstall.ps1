$ErrorActionPreference = 'Stop'

# The installer's own uninstaller, found through its entry in Apps & features.
[array]$keys = Get-UninstallRegistryKey -SoftwareName 'Bifurc'

if ($keys.Count -eq 1) {
  $uninstaller = $keys[0].UninstallString.Trim('"')
  Uninstall-ChocolateyPackage -PackageName $env:ChocolateyPackageName -FileType 'exe' -SilentArgs '/S' -ValidExitCodes @(0) -File $uninstaller
} elseif ($keys.Count -eq 0) {
  Write-Warning "Bifurc is not in Apps & features: it was already uninstalled."
} else {
  Write-Warning "$($keys.Count) entries match Bifurc in Apps & features; none was uninstalled."
  $keys | ForEach-Object { Write-Warning "- $($_.DisplayName)" }
}
