param([string]$Destination = (Join-Path $PSScriptRoot '../../outputs/SetupScope-local'))
$ErrorActionPreference = 'Stop'
$packageSourceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$packageDestination = [IO.Path]::GetFullPath($Destination)
$packageExe = Join-Path $packageSourceRoot 'target/release/setupscope.exe'
if (-not (Test-Path -LiteralPath $packageExe)) { throw 'Build the release executable first.' }
$packageCli = Join-Path $packageSourceRoot 'target/release/setupscope-cli.exe'
if (-not (Test-Path -LiteralPath $packageCli)) { throw 'Build the release CLI executable first.' }
if (Get-Process SetupScope,SetupScope-cli -ErrorAction SilentlyContinue) { throw 'Close SetupScope before packaging.' }
New-Item -ItemType Directory -Force -Path $packageDestination | Out-Null
foreach ($packageFile in @('LICENSE','README.md','CHANGELOG.md')) {
    Copy-Item -LiteralPath (Join-Path $packageSourceRoot $packageFile) -Destination $packageDestination -Force
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Test-CleanMachine.ps1') -Destination $packageDestination -Force
Copy-Item -LiteralPath $packageExe -Destination (Join-Path $packageDestination 'SetupScope.exe') -Force
Copy-Item -LiteralPath $packageCli -Destination (Join-Path $packageDestination 'SetupScope-cli.exe') -Force
New-Item -ItemType File -Force -Path (Join-Path $packageDestination 'portable') | Out-Null
foreach ($packageFolder in @('assets','docs')) {
    $packageTargetFolder=Join-Path $packageDestination $packageFolder
    New-Item -ItemType Directory -Force -Path $packageTargetFolder | Out-Null
    Copy-Item -Path (Join-Path $packageSourceRoot "$packageFolder/*") -Destination $packageTargetFolder -Recurse -Force
}
& python (Join-Path $PSScriptRoot 'release-metadata.py') $packageDestination
if ($LASTEXITCODE -ne 0) {throw 'Dependency inventory generation failed.'}
$packageHashes=@()
Get-ChildItem -LiteralPath $packageDestination -File -Recurse | ForEach-Object {
    $packageRelative=$_.FullName.Substring($packageDestination.TrimEnd([IO.Path]::DirectorySeparatorChar).Length+1).Replace('\','/')
    if ($packageRelative -notlike 'SetupScopeData/*' -and $packageRelative -ne 'SHA256SUMS.txt') {
        $packageHashes+=('{0}  {1}' -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(),$packageRelative)
    }
}
$packageHashes | Sort-Object | Set-Content -LiteralPath (Join-Path $packageDestination 'SHA256SUMS.txt') -Encoding utf8
Write-Output "Current package: $packageDestination"
Write-Output ('Executable SHA256: '+(Get-FileHash -LiteralPath (Join-Path $packageDestination 'SetupScope.exe') -Algorithm SHA256).Hash)
