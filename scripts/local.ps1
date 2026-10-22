param(
    [ValidateSet('build', 'run', 'test', 'test-native', 'check', 'release', 'preview')]
    [string]$Task = 'build'
)

$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Rust is required. Install stable Rust with the Windows MSVC toolchain, then reopen PowerShell.'
    }
    switch ($Task) {
        'build'   { & cargo build --locked }
        'run'     { & cargo run --locked }
        'release' { & cargo build --release --locked }
        'preview' { & cargo run --locked --features native-ui-preview }
        'check'   { & cargo clippy --all-targets --locked }
        'test'    {
            & cargo build --example lockholder --locked
            if ($LASTEXITCODE -ne 0) { throw "Test helper build failed (exit $LASTEXITCODE)." }
            & cargo test --locked
        }
        'test-native' { & cargo test --bin setupscope --features native-ui-preview --locked }
    }
    if ($LASTEXITCODE -ne 0) { throw "Local task '$Task' failed (exit $LASTEXITCODE)." }
} finally {
    Pop-Location
}
