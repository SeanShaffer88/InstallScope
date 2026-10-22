<#
.SYNOPSIS
  Soak test: install, remove and verify real programs over and over for
  hours or days, with SetupScope's background watcher running throughout.

.DESCRIPTION
  Each cycle installs one program under monitoring, removes it with SetupScope
  (closing it if it started, restarting Explorer if needed), and checks that
  nothing it added is left. Meanwhile `setupscope watch` runs as it does after
  sign-in, and its handle count and memory are sampled every cycle.

  Fails (exit 1) when:
    - a removal reports a failure, or leaves anything behind;
    - the watcher records an install log of its own for an installation
      manual monitoring already recorded, or stops running;
    - after warming up, the watcher's handles or memory keep growing past
      the limits below (a leak).

  Results go to soak.csv (one row per cycle) and soak-summary.md in -Work.
  Run it in a disposable VM: it really installs and removes programs.

.EXAMPLE
  # Three days on a test VM, from an elevated PowerShell:
  .\scripts\soak.ps1 -SetupScope .\target\release\setupscope.exe -Hours 72
#>
param(
  [Parameter(Mandatory)] [string] $SetupScope,
  [double] $Hours = 5,
  [string] $Work = "$env:TEMP\setupscope-soak",
  # Cycles before the watcher's resource use is taken as the baseline.
  [int] $WarmupCycles = 3,
  # Allowed growth over the baseline before it counts as a leak.
  [int] $MaxHandleGrowth = 300,
  [int] $MaxMemoryGrowthMB = 150,
  # Minutes one installation or removal may take.
  [int] $StepMinutes = 20
)

$ErrorActionPreference = 'Stop'

# Programs the hosted runner images don't have, one per installer type.
$installers = @(
  @{ name = 'KeePassXC (MSI)'; file = 'kp.msi'; exe = 'msiexec'; args = @('/i', '{file}', '/qn', '/norestart')
     url = 'https://github.com/keepassxreboot/keepassxc/releases/download/2.7.9/KeePassXC-2.7.9-Win64.msi' },
  @{ name = 'Notepad++ (NSIS)'; file = 'npp.exe'; exe = '{file}'; args = @('/S')
     url = 'https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v8.7.1/npp.8.7.1.Installer.x64.exe' },
  @{ name = 'Inno Setup (Inno)'; file = 'is.exe'; exe = '{file}'; args = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')
     url = 'https://github.com/jrsoftware/issrc/releases/download/is-6_4_3/innosetup-6.4.3.exe' }
)

New-Item -ItemType Directory -Force $Work, "$Work\cache", "$Work\app\SetupSweepData" | Out-Null
$exe = "$Work\app\SetupScope.exe"
Copy-Item $SetupScope $exe -Force
# Portable mode keeps logs and backups in the work folder.
New-Item "$Work\app\portable" -ItemType File -Force | Out-Null
@{
  first_run_done = $true
  registry_roots = @('HKLM\SOFTWARE', 'HKLM\SYSTEM\CurrentControlSet\Services', 'HKCU\Software', 'HKCU\Environment')
  file_roots = @(
    @{ path = '%ProgramFiles%'; recursive = $true },
    @{ path = '%ProgramFiles(x86)%'; recursive = $true },
    @{ path = '%ProgramData%\Microsoft\Windows\Start Menu'; recursive = $true },
    @{ path = '%APPDATA%\Microsoft\Windows\Start Menu'; recursive = $true },
    @{ path = '%PUBLIC%\Desktop'; recursive = $false },
    @{ path = '%USERPROFILE%\Desktop'; recursive = $false }
  )
} | ConvertTo-Json -Depth 5 | Set-Content "$Work\app\SetupSweepData\settings.json"

foreach ($i in $installers) {
  $out = "$Work\cache\$($i.file)"
  if (-not (Test-Path $out)) {
    Write-Host "Downloading $($i.name)"
    Invoke-WebRequest -Uri $i.url -OutFile $out -MaximumRedirection 10
  }
}

# Run SetupScope and wait for it alone (not for programs an installer leaves
# running). Returns its exit code, standard output and error output.
function Invoke-SetupScope([string[]] $argv, [int] $minutes = $StepMinutes) {
  $quoted = $argv | ForEach-Object { if ($_ -match ' ') { '"' + $_ + '"' } else { $_ } }
  $p = Start-Process $exe -ArgumentList $quoted -PassThru -NoNewWindow `
    -RedirectStandardOutput "$Work\out.txt" -RedirectStandardError "$Work\err.txt"
  $null = $p.Handle
  if (-not $p.WaitForExit($minutes * 60 * 1000)) {
    Stop-Process -Id $p.Id -Force
    return @{ code = -1; out = ''; err = "timed out after $minutes minutes" }
  }
  return @{ code = $p.ExitCode; out = [string](Get-Content "$Work\out.txt" -Raw); err = [string](Get-Content "$Work\err.txt" -Raw) }
}

# Install logs background monitoring recorded on its own.
function Get-AutoLogs {
  $r = Invoke-SetupScope @('logs', '--tag', 'auto', '--json') 2
  if ($r.code -ne 0 -or -not $r.out) { return @() }
  @($r.out | ConvertFrom-Json)
}

$watch = Start-Process $exe -ArgumentList 'watch' -PassThru
$null = $watch.Handle
Start-Sleep -Seconds 60   # let it take its baseline

$csv = "$Work\soak.csv"
'cycle,time,program,ok,install_s,remove_s,detail,watch_handles,watch_private_mb,data_mb' | Set-Content $csv
$deadline = (Get-Date).AddHours($Hours)
$failures = New-Object System.Collections.Generic.List[string]
$baseline = $null
$cycle = 0

while ((Get-Date) -lt $deadline -and $failures.Count -eq 0) {
  $cycle++
  $i = $installers[($cycle - 1) % $installers.Count]
  $file = "$Work\cache\$($i.file)"
  $iexe = $i.exe.Replace('{file}', $file)
  $iargs = $i.args | ForEach-Object { $_.Replace('{file}', $file) }
  $ok = $true; $detail = ''

  $t = [Diagnostics.Stopwatch]::StartNew()
  $r = Invoke-SetupScope (@('monitor', '--no-prompt', '--json', '--', $iexe) + $iargs)
  $installS = [math]::Round($t.Elapsed.TotalSeconds)
  $removeS = 0
  if ($r.code -ne 0) {
    $ok = $false; $detail = "monitor failed ($($r.code)): " + (($r.err -split "`n" | Select-Object -Last 3) -join ' | ')
  } else {
    $log = $r.out | ConvertFrom-Json
    $t.Restart()
    $r = Invoke-SetupScope @('revert', $log.id, '--close-programs', '--restart-explorer', '--keep-log')
    $removeS = [math]::Round($t.Elapsed.TotalSeconds)
    if ($r.code -ne 0) {
      $ok = $false; $detail = 'revert: ' + ((($r.out + $r.err) -split "`n" | Select-String 'FAIL' | Select-Object -First 3) -join ' | ')
    } else {
      $r = Invoke-SetupScope @('check-clean', $log.id)
      if ($r.code -ne 0) {
        $ok = $false; $detail = 'leftovers: ' + ((($r.out + $r.err) -split "`n" | Where-Object { $_ } | Select-Object -First 5) -join ' | ')
      }
    }
    # The log has served its purpose; removing it keeps the store small.
    Remove-Item "$Work\app\SetupSweepData\logs\$($log.id).json" -ErrorAction SilentlyContinue
  }

  $auto = Get-AutoLogs
  if ($auto.Count -gt 0) {
    $ok = $false; $detail += " watcher recorded its own log(s): $(($auto | ForEach-Object { $_.name }) -join ', ')"
  }

  $watch.Refresh()
  if ($watch.HasExited) {
    $ok = $false; $detail += " watcher exited ($($watch.ExitCode))"
    $handles = 0; $privMb = 0
  } else {
    $handles = $watch.HandleCount
    $privMb = [math]::Round($watch.PrivateMemorySize64 / 1MB, 1)
  }
  $dataMb = [math]::Round(((Get-ChildItem "$Work\app\SetupSweepData" -Recurse -File | Measure-Object Length -Sum).Sum) / 1MB, 1)

  if ($cycle -eq $WarmupCycles) { $baseline = @{ handles = $handles; mb = $privMb } }
  if ($baseline -and -not $watch.HasExited) {
    if ($handles -gt $baseline.handles + $MaxHandleGrowth) {
      $ok = $false; $detail += " watcher handles grew from $($baseline.handles) to $handles"
    }
    if ($privMb -gt $baseline.mb + $MaxMemoryGrowthMB) {
      $ok = $false; $detail += " watcher memory grew from $($baseline.mb) MB to $privMb MB"
    }
  }

  $line = '{0},{1:s},"{2}",{3},{4},{5},"{6}",{7},{8},{9}' -f $cycle, (Get-Date), $i.name, $ok, $installS, $removeS, $detail.Replace('"', "'"), $handles, $privMb, $dataMb
  Add-Content $csv $line
  Write-Host $line
  if (-not $ok) { $failures.Add("cycle $cycle ($($i.name)): $detail") }
}

if (-not $watch.HasExited) { Stop-Process -Id $watch.Id -Force }

$rows = Import-Csv $csv
$summary = @"
### Soak test
| Measure | Value |
|---|---|
| Cycles | $($rows.Count) |
| Hours | $([math]::Round(((Get-Date) - $deadline.AddHours(-$Hours)).TotalHours, 1)) |
| Failures | $($failures.Count) |
| Watcher handles (first, last) | $($rows[0].watch_handles), $($rows[-1].watch_handles) |
| Watcher private memory MB (first, last) | $($rows[0].watch_private_mb), $($rows[-1].watch_private_mb) |
| SetupScope data MB (last) | $($rows[-1].data_mb) |
"@
if ($failures.Count) { $summary += "`n**First failure:** $($failures[0])`n" }
$summary | Set-Content "$Work\soak-summary.md"
Write-Host $summary
if ($failures.Count) { Write-Host "::error title=Soak test::$($failures[0])"; exit 1 }
