//! LSP capability implementations (one module per feature)
//!
//! Phase 0 contains existing capabilities:
//! - [`diag`]      — textDocument/publishDiagnostics (push)
//! - [`gotodef`]   — textDocument/definition
//! - [`refs`]      — textDocument/references
//! - [`semtok`]    — textDocument/semanticTokens/full + range
//!
//! Phase 4 additions:
//! - [`comp`]      — textDocument/completion (auto-completion)
//! - [`fmt`]       — textDocument/formatting (code formatting)
//! - [`inhint`]    — textDocument/inlayHint (inline hints)
//!
//! Subsequent phase additions: hover / rename, etc.
//! Detailed plan see `doc/features.md`.

pub mod comp;
pub mod diag;
pub mod fmt;
pub mod gotodef;
pub mod hover;
pub mod inhint;
pub mod refs;
pub mod semtok;
pub mod usejump;
