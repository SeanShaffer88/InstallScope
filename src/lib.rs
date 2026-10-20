//! InstallScope core library: snapshots, diffs, precise uninstall plans, backups,
//! leftover scanning, startup management, cleanup, and reports.
//!
//! Copyright (C) 2026 Sean Shaffer
//!
//! This program is free software: you can redistribute it and/or modify it
//! under the terms of the GNU General Public License as published by the Free
//! Software Foundation, either version 3 of the License, or (at your option)
//! any later version. See LICENSE for details.

pub mod alternatives;
pub mod autoruns;
pub mod autowatch;
pub mod backup;
pub mod boot;
pub mod cache;
pub mod canary;
pub mod cleaner;
pub mod debloat;
pub mod decrap;
pub mod deflate;
pub mod difftree;
pub mod extinv;
pub mod features;
pub mod fswalk;
pub mod history;
pub mod i18n;
pub mod json;
pub mod leftovers;
pub mod live;
pub mod llm;
pub mod locks;
pub mod merge;
pub mod model;
pub mod nlquery;
pub mod noise;
pub mod orphans;
pub mod pafbuild;
pub mod patools;
pub mod platform;
pub mod png;
pub mod portability;
pub mod portable;
pub mod prescreen;
pub mod programs;
pub mod refcount;
pub mod repackage;
pub mod report;
pub mod restore;
pub mod revert;
pub mod rewind;
pub mod schedule;
pub mod settings;
pub mod shell;
pub mod shellext;
pub mod silent;
pub mod slimmer;
pub mod snapshot;
pub mod startup;
pub mod store;
pub mod time;
pub mod transfer;
pub mod trust;
pub mod usage;
pub mod util;
pub mod zip;

#[cfg(test)]
mod json_compat_tests;
#[cfg(all(test, windows))]
mod lock_tests;
#[cfg(test)]
mod stress_tests;

pub const APP_NAME: &str = "InstallScope";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_AUTHOR: &str = "Sean Shaffer";
