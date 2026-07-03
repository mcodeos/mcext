//! Diagnostics — Syntax/semantic diagnostics (DEPRECATED)
//!
//! This module is deprecated. Diagnostics are now fetched via RPC in `server/mod.rs::parse_and_publish`.
//!
//! LSP entry point: `textDocument/publishDiagnostics` (push mode)
//! Data source: `diagnostics` RPC method

// This module is kept for reference but is no longer used.
// All diagnostics logic has been moved to server/mod.rs which fetches via RPC.
