//! Diagnostics — Syntax/semantic diagnostics
//!
//! LSP entry point: `textDocument/publishDiagnostics` (push mode)
//! Data source: `mcc::mcc_diagnose(uri) -> Vec<Diagnostic>`
//!
//! This module only does **pure conversion**: translates mcc's `Diagnostic` to LSP's `Diagnostic`.
//! Publishing, debounce, version validation, etc. are done in `server/mod.rs::Backend::publish_diagnostics`
//! (Phase 2 adds debounce).

use crate::common::position::offset_to_position;
use crate::state::WorkspaceState;
use mcc::{DiagnosticLevel, McDiagnostic, McURI};
use ropey::Rope;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range, Url};

/// Collect LSP diagnostics for document corresponding to URI.
///
/// If document doesn't exist in state, returns empty Vec.
pub fn collect(state: &WorkspaceState, uri: &Url) -> Vec<Diagnostic> {
    let rope = match state.document_rope(uri) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let mc_uri = match uri_to_mc_uri(uri) {
        Some(u) => u,
        None => return Vec::new(),
    };

    mcc::mcc_diagnose(&mc_uri)
        .into_iter()
        .filter_map(|item| convert_one(item, &rope))
        .collect()
}

fn convert_one(item: McDiagnostic, rope: &Rope) -> Option<Diagnostic> {
    let start: Position = offset_to_position(item.loc.pos as usize, rope)?;
    let end: Position = offset_to_position((item.loc.pos + item.loc.len) as usize, rope)?;
    let severity = match item.level {
        DiagnosticLevel::Error => Some(DiagnosticSeverity::ERROR),
        DiagnosticLevel::Warning => Some(DiagnosticSeverity::WARNING),
        DiagnosticLevel::Info => Some(DiagnosticSeverity::INFORMATION),
        DiagnosticLevel::Hint => Some(DiagnosticSeverity::HINT),
    };

    Some(Diagnostic::new(
        Range::new(start, end),
        severity,
        Some(NumberOrString::Number(item.code as i32)),
        Some("mcc".into()),
        item.msg,
        None,
        None,
    ))
}

/// Url → McURI conversion
///
/// mcc::McURI is typically a string (path), LSP uses `Url`. Here we directly use URL's path
/// part as McURI; specific normalization (absolute path, symlink resolution) done at higher layer.
fn uri_to_mc_uri(uri: &Url) -> Option<McURI> {
    Some(McURI::from(uri.path()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcc::{DiagnosticLevel, McDiagnostic, McLocation};
    use tower_lsp::lsp_types::Position;

    fn make_diag(code: u32, level: DiagnosticLevel, pos: u32, len: u32) -> McDiagnostic {
        McDiagnostic {
            code,
            level,
            loc: McLocation {
                uri: McURI::from("/test.mc"),
                pos,
                len,
                row: 0,
                col: 0,
            },
            msg: "test message".to_string(),
            other: vec![],
        }
    }

    #[test]
    fn converts_error_level() {
        let rope = Rope::from_str("hello");
        let d = make_diag(101, DiagnosticLevel::Error, 0, 5);
        let out = convert_one(d, &rope).unwrap();
        assert_eq!(out.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(out.range.start, Position::new(0, 0));
        assert_eq!(out.range.end, Position::new(0, 5));
        assert_eq!(out.message, "test message");
        assert_eq!(out.source.as_deref(), Some("mcc"));
    }

    #[test]
    fn converts_warning_level() {
        let rope = Rope::from_str("abc\ndef");
        let d = make_diag(202, DiagnosticLevel::Warning, 4, 3);
        let out = convert_one(d, &rope).unwrap();
        assert_eq!(out.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(out.range.start, Position::new(1, 0));
    }

    #[test]
    fn converts_info_and_hint() {
        let rope = Rope::from_str("xx");
        for (lvl, expected) in [
            (DiagnosticLevel::Info, DiagnosticSeverity::INFORMATION),
            (DiagnosticLevel::Hint, DiagnosticSeverity::HINT),
        ] {
            let d = make_diag(303, lvl, 0, 2);
            let out = convert_one(d, &rope).unwrap();
            assert_eq!(out.severity, Some(expected));
        }
    }

    #[test]
    fn out_of_bounds_offset_returns_none() {
        let rope = Rope::from_str("abc");
        let d = make_diag(404, DiagnosticLevel::Error, 10, 1);
        let out = convert_one(d, &rope);
        assert!(out.is_none());
    }
}
