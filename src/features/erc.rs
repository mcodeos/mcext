//! ERC (electrical rule check) — mcc `erc` RPC consumed as diagnostics.
//!
//! The checks are workspace-level (they run the flat net build of the top
//! module), so they cannot ride the per-document `diagnostics` RPC. Instead
//! [`run_and_cache`] is triggered by an explicit save or `mcode.buildProject`,
//! stores the violations grouped by file in [`crate::state::ErcState`], and
//! every publish merges the cache under the source tag `mcc-erc`.

use crate::mccsrv::MccServer;
use crate::state::WorkspaceState;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Range, Url};

/// The diagnostics source tag for ERC violations (vs "mcc" for parse diags).
pub const ERC_SOURCE: &str = "mcc-erc";

/// Run the `erc` RPC and cache its violations grouped by file URI.
///
/// Returns the violation count. A failed RPC or a violation whose file is
/// neither open nor readable contributes nothing — the previous cache for the
/// untouched files survives, stale entries for files that no longer violate
/// are dropped.
pub async fn run_and_cache(state: &WorkspaceState, server: &MccServer) -> usize {
    let resp = match server.erc().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("erc RPC failed: {e} — keeping previous ERC cache");
            return state.erc.diags.iter().map(|d| d.value().len()).sum();
        }
    };

    // Regroup fresh violations by file, then swap the whole cache in one go so
    // files that no longer violate lose their rows.
    let mut fresh: std::collections::BTreeMap<Url, Vec<Diagnostic>> =
        std::collections::BTreeMap::new();
    let total = resp.violations.len();
    for v in resp.violations {
        let uri = match Url::from_file_path(&v.uri) {
            Ok(u) => u,
            Err(_) => match Url::parse(&v.uri) {
                Ok(u) => u,
                Err(_) => {
                    tracing::warn!("erc violation with unparseable uri {} — dropped", v.uri);
                    continue;
                }
            },
        };
        let Some(rope) = state.rope_for_uri(&uri) else {
            tracing::debug!(
                "erc violation in unreadable file {} — dropped",
                uri.path()
            );
            continue;
        };
        let Some(start) = crate::common::position::offset_to_position(v.pos, &rope) else {
            continue;
        };
        // mcc renders ERC anchors as a point (the violation carries no length);
        // underline to the end of the line so the squiggle is visible.
        let end = crate::common::position::line_end_position(start.line, &rope);
        let severity = match v.severity.as_str() {
            "warning" => DiagnosticSeverity::WARNING,
            "info" => DiagnosticSeverity::INFORMATION,
            "hint" => DiagnosticSeverity::HINT,
            _ => DiagnosticSeverity::ERROR,
        };
        let diag = Diagnostic::new(
            Range::new(start, end),
            Some(severity),
            Some(NumberOrString::Number(v.code as i32)),
            Some(ERC_SOURCE.into()),
            v.message,
            None,
            None,
        );
        fresh.entry(uri).or_default().push(diag);
    }

    state.erc.diags.retain(|k, _| fresh.contains_key(k));
    for (uri, diags) in fresh {
        state.erc.diags.insert(uri, diags);
    }
    total
}

/// Re-publish parse ∪ ERC for every URI the latest run touched or that has a
/// cached parse side — the caller refreshed the ERC cache and the Problems
/// panel must follow without waiting for the next parse.
pub async fn republish_affected(
    state: &WorkspaceState,
    client: &tower_lsp::Client,
) {
    let mut uris: Vec<Url> = state.erc.diags.iter().map(|e| e.key().clone()).collect();
    for e in state.erc.last_parse.iter() {
        uris.push(e.key().clone());
    }
    uris.sort();
    uris.dedup();
    for uri in uris {
        let version = state.document_version(&uri);
        let merged = state.erc.merged_for(&uri, Vec::new());
        client.publish_diagnostics(uri, merged, version).await;
    }
}

/// Byte-offset anchor of a violation → LSP position (test helper surface).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::ErcViolation;
    use tower_lsp::lsp_types::Position;

    /// Byte-offset anchor of a violation → LSP position.
    fn position_at(rope: &ropey::Rope, offset: usize) -> Option<Position> {
        crate::common::position::offset_to_position(offset, rope)
    }

    fn violation(pos: usize, uri: &str, severity: &str) -> ErcViolation {
        ErcViolation {
            code: 5003,
            severity: severity.into(),
            check: "multi_drive".into(),
            message: "net N1 is multi-driven".into(),
            net_name: "N1".into(),
            pos,
            uri: uri.into(),
        }
    }

    fn lsp_severity(severity: &str) -> DiagnosticSeverity {
        match severity {
            "warning" => DiagnosticSeverity::WARNING,
            "info" => DiagnosticSeverity::INFORMATION,
            "hint" => DiagnosticSeverity::HINT,
            _ => DiagnosticSeverity::ERROR,
        }
    }

    /// The violation → diagnostic mapping: severity tiers, source tag, code
    /// passthrough, and the byte offset landing on the right line/column with
    /// the range extending to end of line.
    #[test]
    fn violation_maps_to_lsp_diagnostic() {
        let text = "module t;\n  net N1;\n  net N2;\n";
        let rope = ropey::Rope::from_str(text);
        let start = position_at(&rope, 13).unwrap(); // the 'e' of "net" on line 2
        assert_eq!((start.line, start.character), (1, 3));
        let end = crate::common::position::line_end_position(start.line, &rope);
        assert_eq!(end.line, 1);
        assert_eq!(end.character, 9); // "  net N1;" is 9 chars

        let v = violation(13, "/tmp/x.mc", "warning");
        let diag = Diagnostic::new(
            Range::new(start, end),
            Some(lsp_severity(&v.severity)),
            Some(NumberOrString::Number(v.code as i32)),
            Some(ERC_SOURCE.into()),
            v.message,
            None,
            None,
        );
        assert_eq!(diag.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diag.source.as_deref(), Some("mcc-erc"));
        assert_eq!(diag.code, Some(NumberOrString::Number(5003)));
    }

    /// Unknown severity strings collapse to ERROR, matching the parse-diag
    /// convention in parse_and_publish.
    #[test]
    fn unknown_severity_is_error() {
        assert_eq!(lsp_severity("bogus"), DiagnosticSeverity::ERROR);
        assert_eq!(lsp_severity(""), DiagnosticSeverity::ERROR);
        assert_eq!(lsp_severity("error"), DiagnosticSeverity::ERROR);
        assert_eq!(lsp_severity("hint"), DiagnosticSeverity::HINT);
    }
}
