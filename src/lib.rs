//! mcext: VSCode extension for mcode language (LSP server)
//!
//! Library entry: exposes `mcodels` for external use (CLI, other Rust crates).
//! Binary entry point see `main.rs`.

pub mod common;
pub mod features;
pub mod index;
pub mod mcclock;
pub mod mccsrv;
pub mod project;
pub mod rpc;
pub mod server;
pub mod state;
pub mod util;

pub use server::Backend;
pub use state::WorkspaceState;
