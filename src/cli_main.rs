//! Console-subsystem entry point: reliable waiting, redirection and exit codes.
// Only the free snapshot function is used by this entry point.
#[allow(dead_code)]
#[path = "gui/system_info.rs"]
mod system_info;
mod toolkit_cli;
fn main() {
    #[cfg(windows)]
    setupscope_core::platform::windows::quiet_critical_errors();
    setupscope_core::i18n::set_language("");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(build) = setupscope_core::platform::unsupported_windows() {
        eprintln!("SetupScope requires Windows 11; detected build {build}");
        std::process::exit(1);
    }
    std::process::exit(toolkit_cli::dispatch(&args));
}
