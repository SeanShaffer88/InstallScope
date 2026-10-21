// SetupSweep — Windows Install, Uninstall & Developer Tools.
// Copyright (C) 2026 Sean Shaffer
//
// This program is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option)
// any later version. This program is distributed WITHOUT ANY WARRANTY.
// See the LICENSE file for details.

// No console window for the GUI; the CLI attaches to the parent console.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod cli;
mod gui;

fn main() {
    #[cfg(windows)]
    setupsweep_core::platform::windows::quiet_critical_errors();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // "Install with SetupSweep" from Explorer opens the window, not the CLI.
    let install_target = setupsweep_core::shell::monitor_target(&args);
    // SetupSweep is engineered for Windows 11 only: its tweaks, cleanup and
    // removal rules assume Windows 11 locations and behaviour.
    if let Some(build) = setupsweep_core::platform::unsupported_windows() {
        setupsweep_core::i18n::set_language("");
        let msg = setupsweep_core::i18n::tf(
            "SetupSweep requires Windows 11. This PC runs an older Windows (build {build}).",
            &[("build", build.to_string())],
        );
        #[cfg(windows)]
        {
            if args.is_empty() || install_target.is_some() {
                let _ = setupsweep_core::platform::windows::message_box("SetupSweep", &msg);
            } else {
                setupsweep_core::platform::windows::attach_parent_console();
            }
        }
        eprintln!("{msg}");
        std::process::exit(1);
    }
    if !args.is_empty() && install_target.is_none() {
        #[cfg(windows)]
        setupsweep_core::platform::windows::attach_parent_console();
        setupsweep_core::i18n::set_language("");
        std::process::exit(cli::run(args));
    }
    if let Err(e) = gui::run(install_target, args) {
        #[cfg(windows)]
        {
            let msg = format!("SetupSweep could not start its window: {e}\n\nThis usually means the graphics driver is missing or broken.");
            let _ = setupsweep_core::platform::windows::message_box("SetupSweep", &msg);
        }
        eprintln!("{e}");
        std::process::exit(1);
    }
}
