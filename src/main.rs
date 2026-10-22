// SetupScope — Windows Install, Uninstall & Developer Tools.
// Copyright (C) 2026 Sean Shaffer
//
// Licensed under the MIT License. See the LICENSE file for details.

// No console window for the GUI; the CLI attaches to the parent console.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod gui;
mod instance;
mod toolkit_cli;
use gui::system_info;

fn main() {
    #[cfg(windows)]
    setupscope_core::platform::windows::quiet_critical_errors();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Retired installer arguments are rejected before GUI startup.
    let install_target: Option<String> = None;
    // Supported desktop host: Windows 11.
    if let Some(build) = setupscope_core::platform::unsupported_windows() {
        setupscope_core::i18n::set_language("");
        let msg = setupscope_core::i18n::tf(
            "SetupScope requires Windows 11. This PC runs an older Windows (build {build}).",
            &[("build", build.to_string())],
        );
        #[cfg(windows)]
        {
            if args.is_empty() || install_target.is_some() {
                let _ = setupscope_core::platform::windows::message_box("SetupScope", &msg);
            } else {
                setupscope_core::platform::windows::attach_parent_console();
            }
        }
        eprintln!("{msg}");
        std::process::exit(1);
    }
    if !args.is_empty() && install_target.is_none() {
        #[cfg(windows)]
        setupscope_core::platform::windows::attach_parent_console();
        setupscope_core::i18n::set_language("");
        let code = toolkit_cli::dispatch(&args);
        std::process::exit(code);
    }
    #[cfg(windows)]
    let _instance = match instance::Instance::acquire() {
        Ok(Some(guard)) => guard,
        Ok(None) => {instance::focus_existing();return;},
        Err(e) => {
            let _ = setupscope_core::platform::windows::message_box("SetupScope", &format!("Cannot establish the single-instance guard: {e}. No additional window was opened."));
            std::process::exit(1);
        }
    };
    if let Err(e) = gui::run(install_target, args) {
        #[cfg(windows)]
        {
            let msg = format!("SetupScope could not start its window: {e}\n\nThis usually means the graphics driver is missing or broken.");
            let _ = setupscope_core::platform::windows::message_box("SetupScope", &msg);
        }
        eprintln!("{e}");
        std::process::exit(1);
    }
}
