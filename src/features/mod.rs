//! LSP capability implementations (one module per feature)
//!
//! Phase 0 contains existing capabilities:
//! - [`diagnostics`]   — textDocument/publishDiagnostics (push)
//! - [`goto_definition`] — textDocument/definition
//! - [`references`]    — textDocument/references
//! - [`semantic_tokens`] — textDocument/semanticTokens/full + range
//!
//! Phase 4 additions:
//! - [`completion`]     — textDocument/completion (auto-completion)
//! - [`formatting`]    — textDocument/formatting (code formatting)
//! - [`inlay_hint`]    — textDocument/inlayHint (inline hints)
//!
//! Subsequent phase additions: hover / rename, etc.
//! Detailed plan see `doc/features.md`.

pub mod completion;
pub mod diagnostics;
pub mod formatting;
pub mod goto_definition;
pub mod inlay_hint;
pub mod references;
pub mod semantic_tokens;
