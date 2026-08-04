//! Core library for tfrm: HCP Terraform API client, plan model, diff and
//! redaction. Kept library-shaped so tests drive the logic without clap.

pub mod credentials;
pub mod error;

pub use error::{Error, Result};
