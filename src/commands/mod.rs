//! Top-level subcommand entry points routed from `main.rs`.
//!
//! Each submodule corresponds to one `colorant` subcommand and exposes a
//! single `run` function. The orchestration logic (config, mode detection,
//! OSC emission) lives in `apply`; the others are thin wrappers.

pub mod apply;
pub mod current;
pub mod doctor;
pub mod init;
pub mod reset;
pub mod set;
pub mod show;
pub mod themes;
