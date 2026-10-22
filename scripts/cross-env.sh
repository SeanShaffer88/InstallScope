# Environment for cross-compiling SetupScope to Windows from Linux without
# Visual Studio (MinGW-w64 linker, std built from source).
# Usage: source scripts/cross-env.sh && cargo xbuild-win
export PATH=/usr/lib/rust-1.91/bin:$PATH
export RUSTC_BOOTSTRAP=1
export __CARGO_TESTS_ONLY_SRC_ROOT=/opt/rust-win/lib/rustlib/src/rust/library
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L native=/opt/rust-win/rtobj -C target-feature=+crt-static"
WINFLAGS="-Zbuild-std=std,panic_abort --target x86_64-pc-windows-gnu"
