#requires -Version 7.0
param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference='Stop'
$installerPath=(Resolve-Path -LiteralPath $Installer).Path
$key='HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/SetupScope'
$menu=Join-Path ([Environment]::GetFolderPath('Programs')) 'SetupScope'
$desktop=Join-Path ([Environment]::GetFolderPath('Desktop')) 'SetupScope.lnk'
if ((Test-Path $key) -or (Test-Path -LiteralPath $menu) -or (Test-Path -LiteralPath $desktop)) {
    throw 'An existing SetupScope installation/shortcut must be preserved. Use a separate test account.'
}
$owner=[guid]::NewGuid().ToString('N')
$testRoot=Join-Path ([IO.Path]::GetTempPath()) ('SetupScope-installer-test-'+$owner)
New-Item -ItemType Directory -Path $testRoot | Out-Null
Set-Content -LiteralPath (Join-Path $testRoot '.test-owner') -Value $owner -NoNewline
$destination=Join-Path $testRoot 'installed'
$checks=[Collections.Generic.List[string]]::new()
$dataRoot=Join-Path $env:LOCALAPPDATA 'SetupScope'
function DataHashes {
    if (Test-Path -LiteralPath $dataRoot) {
        Get-ChildItem -LiteralPath $dataRoot -File -Recurse | Sort-Object FullName | ForEach-Object {
            $_.FullName+'='+ (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
        }
    }
}
$before=@(DataHashes)
function Run([string]$Executable,[string[]]$Arguments) {
    $info=[Diagnostics.ProcessStartInfo]::new($Executable)
    $info.UseShellExecute=$false; $info.CreateNoWindow=$true
    $info.RedirectStandardOutput=$true; $info.RedirectStandardError=$true
    if ($Arguments.Count -gt 0 -and ($Arguments[-1].StartsWith('/D=') -or $Arguments[-1].StartsWith('_?='))) {
        # NSIS consumes the entire unquoted tail after /D=, including spaces.
        # ArgumentList would quote the whole /D= argument and hide that switch.
        if ($Arguments[-1].Contains('"') -or $Arguments[-1].Contains("`n")) {throw 'Unsafe fixture path'}
        $info.Arguments=($Arguments -join ' ')
    } else {
        foreach($argument in $Arguments) {$info.ArgumentList.Add($argument)}
    }
    $process=[Diagnostics.Process]::Start($info)
    $stdout=$process.StandardOutput.ReadToEndAsync(); $stderr=$process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(60000)) {throw 'Installer test exceeded 60 seconds; fixture retained for inspection.'}
    $null=$stdout.GetAwaiter().GetResult(); $null=$stderr.GetAwaiter().GetResult()
    $code=$process.ExitCode; $process.Dispose(); return $code
}
function Check([string]$Name,[bool]$Passed) {
    if (-not $Passed) {throw "Installer check failed: $Name; fixture retained at $testRoot"}
    $checks.Add($Name)
}
try {
    Check 'Fresh per-user install' ((Run $installerPath @('/S',('/D='+$destination))) -eq 0)
    Check 'Installed registration' ((Get-ItemProperty $key).InstallLocation -eq $destination)
    Check 'Installed GUI and CLI' ((Test-Path -LiteralPath (Join-Path $destination 'SetupScope.exe')) -and (Test-Path -LiteralPath (Join-Path $destination 'SetupScope-cli.exe')))
    Check 'Installed mode (no portable marker)' (-not(Test-Path -LiteralPath (Join-Path $destination 'portable')))
    Check 'Installed CLI loader' ((Run (Join-Path $destination 'SetupScope-cli.exe') @('--version')) -eq 0)
    foreach($line in [IO.File]::ReadAllLines((Join-Path $destination 'SHA256SUMS.txt'))) {
        if ($line -match '^([a-fA-F0-9]{64})  (.+)$') {
            Check ('Installed checksum: '+$Matches[2]) ((Get-FileHash -LiteralPath (Join-Path $destination $Matches[2])).Hash -eq $Matches[1])
        } else {throw 'Invalid installed manifest'}
    }
    $sentinel=Join-Path $destination 'user-preserved.txt'
    Set-Content -LiteralPath $sentinel 'preserve'
    Check 'Same-version upgrade' ((Run $installerPath @('/S',('/D='+$destination))) -eq 0)
    Check 'Upgrade preserves unknown file' ((Get-Content -LiteralPath $sentinel) -eq 'preserve')
    $mutex=[Threading.Mutex]::new($false,'Local\SetupScope.Desktop.v1')
    try {
        Check 'Running-app install guard' ((Run $installerPath @('/S',('/D='+$destination))) -eq 2)
        Check 'Running-app uninstall guard' ((Run (Join-Path $destination 'Uninstall.exe') @('/S',('_?='+$destination))) -eq 2)
    } finally {$mutex.Dispose()}
    $foreign=Join-Path $testRoot 'foreign'
    New-Item -ItemType Directory -Path $foreign | Out-Null
    Set-Content -LiteralPath (Join-Path $foreign 'keep.txt') 'untouched'
    Check 'Reject unrelated nonempty folder' ((Run $installerPath @('/S',('/D='+$foreign))) -eq 2)
    Check 'Rejected folder unchanged' (@(Get-ChildItem -LiteralPath $foreign).Count -eq 1)
    $portable=Join-Path $testRoot 'portable'
    New-Item -ItemType Directory -Path $portable | Out-Null
    New-Item -ItemType File -Path (Join-Path $portable 'portable') | Out-Null
    Check 'Reject portable destination' ((Run $installerPath @('/S',('/D='+$portable))) -eq 2)
    Check 'Uninstall' ((Run (Join-Path $destination 'Uninstall.exe') @('/S')) -eq 0)
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    while ((Test-Path -LiteralPath (Join-Path $destination 'Uninstall.exe')) -and [DateTime]::UtcNow -lt $deadline) {Start-Sleep -Milliseconds 100}
    Check 'Registration removed' (-not(Test-Path $key))
    Check 'Shortcuts removed' (-not(Test-Path -LiteralPath $menu))
    Check 'Uninstall preserves unknown file' ((Get-Content -LiteralPath $sentinel) -eq 'preserve')
    Check 'Packaged application files removed' (@(Get-ChildItem -LiteralPath $destination -Force).Count -eq 1)
    Check 'Existing user data unchanged' (($before -join "`n") -ceq (@(DataHashes) -join "`n"))
    [pscustomobject]@{Version='1.0.0.0';Passed=$true;Checks=$checks.Count;WorkflowChecks=@($checks | Where-Object {$_ -notlike 'Installed checksum:*'});PayloadChecksums=@($checks | Where-Object {$_ -like 'Installed checksum:*'}).Count;InstallerSHA256=(Get-FileHash -LiteralPath $installerPath).Hash;Limit='Current Windows host and account; silent lifecycle checks, not clean-VM or interactive-page certification.'} | ConvertTo-Json -Depth 4
    if ([IO.Path]::GetDirectoryName($testRoot).TrimEnd('\') -eq [IO.Path]::GetTempPath().TrimEnd('\') -and [IO.File]::ReadAllText((Join-Path $testRoot '.test-owner')) -eq $owner) {
        Remove-Item -LiteralPath $testRoot -Recurse
    }
} catch {
    Write-Error "$_ Fixture retained: $testRoot"
    exit 1
}
