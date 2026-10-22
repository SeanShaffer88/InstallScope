param(
    [string]$Package = $PSScriptRoot,
    [switch]$LaunchGui,
    [switch]$KeepSandbox
)
# Windows PowerShell 5.1+. No SDK, network, administrator or installation needed.
# This is a host test, not an OS VM/security sandbox. No environment is persisted.
$ErrorActionPreference = 'Stop'
# A caller may inherit PowerShell7's module path into Windows PowerShell5.1.
# Load the trusted runtime's own built-ins, never a developer/user module path.
foreach ($module in @('Microsoft.PowerShell.Utility','Microsoft.PowerShell.Management')) {
    Import-Module (Join-Path $PSHOME "Modules/$module/$module.psd1") -ErrorAction Stop
}
$packageRoot = [IO.Path]::GetFullPath($Package)
foreach ($name in @('SetupScope.exe','SetupScope-cli.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $name) -PathType Leaf)) { throw "Missing $name in package." }
}
$hashFile=Join-Path $packageRoot 'SHA256SUMS.txt'
if (-not (Test-Path -LiteralPath $hashFile) -or (Get-Item -LiteralPath $hashFile).Length -gt 1048576) { throw 'Missing or oversized SHA256SUMS.txt.' }
$expectedHashes=@{}
foreach ($line in [IO.File]::ReadAllLines($hashFile)) {
    if ($line -match '^([a-fA-F0-9]{64})  (SetupScope(?:-cli)?\.exe)$') {
        if ($expectedHashes.ContainsKey($Matches[2])) { throw 'Duplicate executable checksum.' }
        $expectedHashes[$Matches[2]]=$Matches[1]
    }
}
foreach ($name in @('SetupScope.exe','SetupScope-cli.exe')) {
    if (-not $expectedHashes.ContainsKey($name) -or (Get-FileHash -LiteralPath (Join-Path $packageRoot $name)).Hash -ne $expectedHashes[$name]) { throw "Executable checksum mismatch: $name" }
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('SetupScope-clean-check-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$marker = [guid]::NewGuid().ToString('N')
[IO.File]::WriteAllText((Join-Path $testRoot '.test-owner'), $marker)
$results = New-Object 'System.Collections.Generic.List[object]'
$failed = $false
$utf8 = New-Object Text.UTF8Encoding($false)
function Assert-Check([string]$Name, [bool]$Condition) {
    if (-not $Condition) { throw "Failed: $Name" }
    $results.Add($Name)
}
function Quote-Argument([string]$Value) {
    # CommandLineToArgvW quoting; no shell is used.
    return '"' + ([regex]::Replace(([regex]::Replace($Value, '(\\*)"', '$1$1\"')), '(\\+)$', '$1$1')) + '"'
}
function Invoke-Probe([string[]]$Arguments) {
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = Join-Path $testRoot 'SetupScope-cli.exe'
    $info.Arguments = (($Arguments | ForEach-Object { Quote-Argument $_ }) -join ' ')
    $info.WorkingDirectory = $testRoot
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.RedirectStandardInput = $true
    # Remove developer/account environment overrides only for this child.
    $info.EnvironmentVariables.Clear()
    foreach ($pair in $isolatedEnvironment.GetEnumerator()) { $info.EnvironmentVariables[$pair.Key] = $pair.Value }
    $child = New-Object Diagnostics.Process
    $child.StartInfo = $info
    try {
        if (-not $child.Start()) { throw 'Cannot start probe' }
        $child.StandardInput.Close()
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit(30000)) {
            # This harness launches only its own bounded CLI probes, no commands.
            $child.Kill(); $child.WaitForExit(); throw 'CLI probe timed out'
        }
        return [pscustomobject]@{ Code=$child.ExitCode; Out=$stdout.Result; Error=$stderr.Result }
    } finally { $child.Dispose() }
}
try {
    foreach ($name in @('SetupScope.exe','SetupScope-cli.exe')) { Copy-Item -LiteralPath (Join-Path $packageRoot $name) -Destination $testRoot }
    foreach ($name in @('SetupScope.exe','SetupScope-cli.exe')) {
        Assert-Check "Copied executable integrity: $name" ((Get-FileHash -LiteralPath (Join-Path $testRoot $name)).Hash -eq $expectedHashes[$name])
    }
    [IO.File]::WriteAllText((Join-Path $testRoot 'portable'), '')
    foreach ($name in @('profile','appdata','localappdata','temp','project')) { New-Item -ItemType Directory -Path (Join-Path $testRoot $name) | Out-Null }
    $isolatedEnvironment = @{
        SystemRoot=$env:SystemRoot; WINDIR=$env:SystemRoot; SystemDrive=$env:SystemDrive;
        PATH=(Join-Path $env:SystemRoot 'System32'); PATHEXT='.COM;.EXE';
        USERPROFILE=(Join-Path $testRoot 'profile'); APPDATA=(Join-Path $testRoot 'appdata');
        LOCALAPPDATA=(Join-Path $testRoot 'localappdata'); TEMP=(Join-Path $testRoot 'temp'); TMP=(Join-Path $testRoot 'temp')
    }
    $r=Invoke-Probe @('--version'); Assert-Check 'Version and loader without developer PATH' ($r.Code -eq 0 -and $r.Out.Trim() -eq 'SetupScope 1.0.0.0')
    $r=Invoke-Probe @('--help'); Assert-Check 'Console help' ($r.Code -eq 0 -and $r.Out.Contains('environment-undo'))
    $r=Invoke-Probe @('languages'); Assert-Check 'Eleven embedded language choices' ($r.Code -eq 0 -and @($r.Out.Trim() -split '\r?\n').Count -eq 11)
    $r=Invoke-Probe @('system','--json'); $system=$r.Out | ConvertFrom-Json
    Assert-Check 'System JSON' ($r.Code -eq 0 -and $system.ok -and $system.command -eq 'system')
    $r=Invoke-Probe @('dependencies','catalog'); $catalog=$r.Out | ConvertFrom-Json
    Assert-Check 'Offline pinned catalog' ($r.Code -eq 0 -and @($catalog.packages).Count -eq 6)
    $project=Join-Path $testRoot 'project'
    [IO.File]::WriteAllText((Join-Path $project 'Cargo.toml'), "[package]`nname = `"clean_check`"`nversion = `"0.1.0`"`nrust-version = `"1.91`"`nbuild = `"build.rs`"`n",$utf8)
    [IO.File]::WriteAllText((Join-Path $project 'build.rs'), 'fn main(){panic!("MUST_NOT_EXECUTE");}',$utf8)
    $before=(Get-FileHash -LiteralPath (Join-Path $project 'Cargo.toml')).Hash
    $r=Invoke-Probe @('readiness',$project,'x64')
    Assert-Check 'Passive project inspection without developer PATH' ($r.Code -eq 0 -and $before -eq (Get-FileHash -LiteralPath (Join-Path $project 'Cargo.toml')).Hash -and @(Get-ChildItem -LiteralPath $project -Force).Count -eq 2)
    $log=Join-Path $testRoot 'synthetic.log'
    [IO.File]::WriteAllText($log,"PRIVATE_TEST_TOKEN`nerror C1083: Cannot open include file: 'fixture.h'`n",$utf8)
    $before=(Get-FileHash -LiteralPath $log).Hash
    $r=Invoke-Probe @('triage',$log)
    Assert-Check 'Read-only synthetic log; raw secret omitted' ($r.Code -eq 0 -and -not $r.Out.Contains('PRIVATE_TEST_TOKEN') -and $before -eq (Get-FileHash -LiteralPath $log).Hash)
    $r=Invoke-Probe @('unknown','--json'); $errorJson=$r.Out | ConvertFrom-Json
    Assert-Check 'Structured failure' ($r.Code -eq 2 -and -not $errorJson.ok)
    $r=Invoke-Probe @('programs'); Assert-Check 'Retired command rejected' ($r.Code -eq 2)
    Assert-Check 'Fresh portable data only' (-not (Test-Path -LiteralPath (Join-Path $testRoot 'lang')))
    if ($LaunchGui) {
        $KeepSandbox=$true
        $guiInfo=New-Object Diagnostics.ProcessStartInfo
        $guiInfo.FileName=Join-Path $testRoot 'SetupScope.exe'; $guiInfo.WorkingDirectory=$testRoot; $guiInfo.UseShellExecute=$false
        $guiInfo.EnvironmentVariables.Clear()
        foreach ($pair in $isolatedEnvironment.GetEnumerator()) { $guiInfo.EnvironmentVariables[$pair.Key]=$pair.Value }
        $gui=[Diagnostics.Process]::Start($guiInfo)
        Write-Output "GUI started as process $($gui.Id). Manually inspect it; startup is not a passed GUI check."
        $gui.Dispose()
    }
} catch {
    $failed=$true; $KeepSandbox=$true
    $results.Add([pscustomobject]@{Failure=$_.Exception.Message})
} finally {
    $report=[pscustomobject]@{
        Schema=1; Version='1.0.0.0'; Passed=(-not $failed); CheckedUtc=[DateTime]::UtcNow.ToString('o');
        GuiRequested=[bool]$LaunchGui; GuiVerified=$false;
        HostBuild=[Environment]::OSVersion.Version.ToString(); Checks=@($results.ToArray());
        Executables=@(Get-FileHash -LiteralPath (Join-Path $packageRoot 'SetupScope.exe'),(Join-Path $packageRoot 'SetupScope-cli.exe') | Select-Object @{Name='Name';Expression={[IO.Path]::GetFileName($_.Path)}},Hash);
        Limit='Per-process environment/profile isolation on the current OS. Not a VM, new user, clean OS, network sandbox, installation or screen-reader certification.'
    }
    $report | ConvertTo-Json -Depth 6 | Write-Output
    if ($KeepSandbox) {
        $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $testRoot 'RESULT.json') -Encoding utf8
        Write-Output "Retained test files: $testRoot"
    } elseif ((Test-Path -LiteralPath (Join-Path $testRoot '.test-owner')) -and [IO.File]::ReadAllText((Join-Path $testRoot '.test-owner')) -eq $marker -and [IO.Path]::GetDirectoryName($testRoot).TrimEnd('\') -eq [IO.Path]::GetTempPath().TrimEnd('\')) {
        Remove-Item -LiteralPath $testRoot -Recurse
    }
}
if ($failed) { exit 1 }
