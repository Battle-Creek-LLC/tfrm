//! Core library for tfrm: HCP Terraform API client, plan model, diff and
//! redaction. Kept library-shaped so tests drive the logic without clap.

pub mod actions;
pub mod client;
pub mod config;
pub mod credentials;
pub mod credfile;
pub mod diff;
pub mod error;
pub mod login;
pub mod plan;
pub mod runs;
pub mod show;
pub mod workspaces;

pub use error::{Error, Result};
