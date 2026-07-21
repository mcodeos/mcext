//! LSP capability implementations (one module per feature)
//!
//! - [`gotodef`]   — textDocument/definition
//! - [`refs`]      — textDocument/references
//! - [`semtok`]    — textDocument/semanticTokens/full + range
//! - [`comp`]      — textDocument/completion (auto-completion)
//! - [`fmt`]       — textDocument/formatting (code formatting)
//! - [`inhint`]    — textDocument/inlayHint (inline hints)
//! - [`hover`]     — textDocument/hover
//! - [`symbols`]   — shared symbol resolution utilities
//!
//! Diagnostics are fetched via RPC in `server/mod.rs::parse_and_publish`.
//! Document links (`usejump`) are disabled — use F12 (goto_definition) instead.

pub mod comp;
pub mod docsym;
pub mod fmt;
pub mod gotodef;
pub mod hover;
pub mod inhint;
pub mod refs;
pub mod semtok;
pub mod symbols;
