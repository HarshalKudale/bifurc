<#
.SYNOPSIS
  Measures install size, cold start to the first window and idle memory of desktop apps, and writes
  docs/assets/bench.json. Run `node scripts/bench/render.mjs` afterwards to draw docs/assets/bench.svg.

.DESCRIPTION
  Each -App is "name|install folder|main executable|process names" (process names are comma-separated and
  all count towards memory). An app that is not installed is left out; nothing is estimated.

  - Cold start: the time from launch until the process has a main window, median of -Runs.
  - Idle memory: the private working set of every process of the app, 30 seconds after the window appeared.
  - Install size: the size of the install folder on disk.

.EXAMPLE
  scripts/bench/bench.ps1 `
    -App 'Bifurc|C:\Program Files\Bifurc|C:\Program Files\Bifurc\bifurc-ui.exe|bifurc-ui,bifurc-engine,bifurc-tray' `
    -App 'Postman|C:\Users\me\AppData\Local\Postman|C:\Users\me\AppData\Local\Postman\Postman.exe|Postman'
#>
param(
  [Parameter(Mandatory)][string[]]$App,
  [int]$Runs = 5,
  [int]$IdleSeconds = 30,
  [string]$Out = (Join-Path (Split-Path $PSScriptRoot -Parent | Split-Path -Parent) 'docs/assets/bench.json')
)
$ErrorActionPreference = 'Stop'

function Stop-App($names) {
  foreach ($n in $names) { Get-Process -Name $n -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 2
}

function PrivateMb($names) {
  $total = 0
  foreach ($n in $names) {
    try {
      $samples = (Get-Counter "\Process($n*)\Working Set - Private" -ErrorAction Stop).CounterSamples
      $total += ($samples | Measure-Object CookedValue -Sum).Sum
    } catch { }
  }
  [math]::Round($total / 1MB, 0)
}

$results = @()
foreach ($spec in $App) {
  $name, $dir, $exe, $procs = $spec.Split('|')
  $names = $procs.Split(',')
  if (-not (Test-Path $exe)) { Write-Warning "$name is not installed at $exe; leaving it out."; continue }
  $sizeMb = [math]::Round((Get-ChildItem $dir -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB, 0)

  $starts = @()
  for ($i = 0; $i -lt $Runs; $i++) {
    Stop-App $names
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $exe -PassThru
    $shown = $false
    while ($watch.Elapsed.TotalSeconds -lt 60) {
      $p.Refresh()
      if ($p.MainWindowHandle -ne 0) { $shown = $true; break }
      Start-Sleep -Milliseconds 20
    }
    if ($shown) { $starts += $watch.ElapsedMilliseconds }
    if ($i -eq $Runs - 1 -and $shown) {
      Start-Sleep -Seconds $IdleSeconds
      $memMb = PrivateMb $names
    }
  }
  Stop-App $names
  if (-not $starts -or -not $memMb) { Write-Warning "$name never showed a window; leaving it out."; continue }
  $sorted = $starts | Sort-Object
  $results += [ordered]@{
    name = $name; installMb = $sizeMb; coldStartMs = $sorted[[int][math]::Floor($sorted.Count / 2)]
    idleMemoryMb = $memMb; runs = $starts.Count
  }
}

$os = (Get-CimInstance Win32_OperatingSystem).Caption
$cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name.Trim()
$ramGb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 0)
$doc = [ordered]@{
  date = (Get-Date -Format 'yyyy-MM-dd'); machine = "$cpu, $ramGb GB RAM"; os = $os; results = $results
}
New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
[IO.File]::WriteAllText($Out, ($doc | ConvertTo-Json -Depth 5) + "`n")
Write-Host "Wrote $Out"
