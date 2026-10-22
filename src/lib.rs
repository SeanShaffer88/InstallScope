//! SetupScope core: installation snapshots/diffs, reviewed uninstall/recovery,
//! developer diagnostics and local project workflows.
//!
//! Copyright (C) 2026 Sean Shaffer
//!
//! Licensed under the MIT License. See LICENSE for details.

pub mod alternatives;
pub mod windowidentify;
pub mod autoruns;
pub mod autowatch;
pub mod backup;
pub mod boot;
pub mod cache;
pub mod canary;
pub mod cleaner;
pub mod cleanup_receipt;
pub mod debloat;
pub mod decrap;
pub mod deflate;
pub mod devcheck;
pub mod dependencies;
pub mod build_log;
pub mod build_run;
pub mod container_readiness;
pub mod diagnostic_bundle;
pub mod devversion;
pub mod devinspect;
pub mod envprofile;
pub mod projectdoctor;
pub mod regression;
pub mod difftree;
pub mod extinv;
pub mod features;
pub mod fswalk;
pub mod gatekeeper;
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
pub mod privacy;
pub mod programs;
pub mod refcount;
pub mod repackage;
pub mod report;
pub mod restore;
pub mod revert;
pub mod rewind;
pub mod schedule;
pub mod settings;
pub mod setup_export;
pub mod shell;
pub mod shellext;
pub mod silent;
pub mod slimmer;
pub mod snapshot;
pub mod startup;
pub mod storage;
pub mod storage_allocation;
pub mod storage_baseline;
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

pub const APP_NAME: &str = "SetupScope";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_AUTHOR: &str = "Sean Shaffer";

pub mod scratch;
pub mod project_readiness;
