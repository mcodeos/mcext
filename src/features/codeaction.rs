//! Code actions — quick-fix surface sourced from the diagnostics already in
//! the request context. Two carriers:
//!
//! * *explain* — run the numeric diagnostic code through the mcc `explain`
//!   RPC (rule owner, acceptance, fix hints, allow-syntax).
//! * *rename* (U327) — style-gate diagnostics whose mcc payload carries a
//!   `fix` edit set become real `WorkspaceEdit` actions: one rename action
//!   per diagnostic, plus a single fix-all that merges every fix in the
//!   context (edits dedup by `(file, line, column)`, so overlapping fixes
//!   collapse rather than double-apply).
//!
//! mcc's `suggestions` are free-text hints that reference locations, not
//! applicable edits, so no `WorkspaceEdit` is fabricated from them.

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Command, Diagnostic, TextEdit, Url,
    WorkspaceEdit,
};

/// Actions for one `textDocument/codeAction` request: one explain action per
/// distinct numeric code, one rename action per fix-carrying diagnostic, and
/// one fix-all when more than one fix is present. Returns an empty vec (not
/// an error) when nothing applies — the editor renders no menu.
pub fn resolve(diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    let mut actions: Vec<CodeActionOrCommand> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    // (file, line, column) → edit; the dedup key for the fix-all merge.
    let mut merged: std::collections::BTreeMap<(String, u32, u32), TextEdit> =
        std::collections::BTreeMap::new();
    let mut fix_count = 0usize;
    for diag in diagnostics {
        if let Some(code) = numeric_code(diag) {
            if !seen.contains(&code) {
                seen.push(code);
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Explain E{code}"),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diag.clone()]),
                    command: Some(Command {
                        title: format!("Explain E{code}"),
                        command: "mcode.explain".to_string(),
                        arguments: Some(vec![serde_json::json!(format!("E{code}"))]),
                    }),
                    ..Default::default()
                }));
            }
        }
        if let Some(fix) = rename_fix(diag) {
            fix_count += 1;
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: fix.title.clone(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                // The rename covers every occurrence, not just the flagged
                // span (exact name compare: a partial rename splits nets).
                edit: Some(workspace_edit(fix.edits.iter().cloned())),
                ..Default::default()
            }));
            for row in fix.edits {
                merged.insert(row.0, row.1);
            }
        }
    }
    // Fix-all: one action applying every fix in the context. Only worth a
    // menu row when it merges at least two fixes.
    if fix_count > 1 && !merged.is_empty() {
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: "Fix all style names in file".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            data: Some(serde_json::json!({ "fixAll": true })),
            edit: Some(workspace_edit(merged.into_iter())),
            ..Default::default()
        }));
    }
    actions
}

/// One rename fix parsed back from a diagnostic's `data.fix` wire payload:
/// its title plus every edit as `(key, TextEdit)`, where the key is
/// `(file, 1-based line, 1-based column)`. `None` when the diagnostic carries
/// no parseable `fix` payload (any other code, or an mcc predating the field).
fn rename_fix(diag: &Diagnostic) -> Option<RenameFix> {
    let fix = diag.data.as_ref()?;
    let title = fix.get("title")?.as_str()?.to_string();
    let edits = fix.get("edits")?.as_array()?;
    let mut out: Vec<((String, u32, u32), TextEdit)> = Vec::new();
    for edit in edits {
        let line = edit.get("line")?.as_u64()? as u32;
        let column = edit.get("column")?.as_u64()? as u32;
        let end_line = edit.get("end_line")?.as_u64()? as u32;
        let end_column = edit.get("end_column")?.as_u64()? as u32;
        if line == 0 || end_line == 0 {
            continue;
        }
        let file = edit
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or_default()
            .to_string();
        let text_edit = TextEdit {
            // mcc lines/columns are 1-based, LSP positions 0-based.
            range: tower_lsp::lsp_types::Range::new(
                tower_lsp::lsp_types::Position::new(line - 1, column.saturating_sub(1)),
                tower_lsp::lsp_types::Position::new(end_line - 1, end_column.saturating_sub(1)),
            ),
            new_text: edit.get("replacement")?.as_str()?.to_string(),
        };
        out.push(((file, line, column), text_edit));
    }
    if out.is_empty() {
        return None;
    }
    Some(RenameFix { title, edits: out })
}

struct RenameFix {
    title: String,
    edits: Vec<((String, u32, u32), TextEdit)>,
}

/// Group keyed edits into a `WorkspaceEdit` by file.
fn workspace_edit(edits: impl Iterator<Item = ((String, u32, u32), TextEdit)>) -> WorkspaceEdit {
    let mut changes: std::collections::HashMap<Url, Vec<TextEdit>> =
        std::collections::HashMap::new();
    for ((file, _, _), edit) in edits {
        if let Ok(url) = Url::from_file_path(&file) {
            changes.entry(url).or_default().push(edit);
        }
    }
    WorkspaceEdit::new(changes)
}

/// The numeric part of a diagnostic code. mcc emits `code` as a number and the
/// published LSP diagnostic carries it as a string (`"E5060"`-style); accept
/// both spellings.
fn numeric_code(diag: &Diagnostic) -> Option<u32> {
    let code = diag.code.as_ref()?;
    match code {
        tower_lsp::lsp_types::NumberOrString::Number(n) => u32::try_from(*n).ok(),
        tower_lsp::lsp_types::NumberOrString::String(s) => {
            let digits = s.trim_start_matches(['E', 'e']);
            digits.parse::<u32>().ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{NumberOrString, Position, Range};

    fn diag(code: NumberOrString) -> Diagnostic {
        Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
            message: "boom".to_string(),
            code: Some(code),
            ..Default::default()
        }
    }

    /// A fix-carrying diagnostic in mcc's wire shape (U327): `data.fix` with
    /// 1-based positions and one edit per occurrence.
    fn fix_diag(name: &str, pos: u32) -> Diagnostic {
        let mut d = diag(NumberOrString::Number(5070));
        d.message = format!("Net/port name '{name}' is not UPPER_SNAKE.");
        d.data = Some(serde_json::json!({
            "title": format!("Rename '{name}' to '{}'", name.to_ascii_uppercase()),
            "edits": [{
                "file": "/proj/a.mc",
                "pos": pos,
                "len": name.len(),
                "line": 3,
                "column": pos,
                "end_line": 3,
                "end_column": pos + name.len() as u32,
                "replacement": name.to_ascii_uppercase(),
            }],
        }));
        d
    }

    #[test]
    fn numeric_string_codes_become_explain_actions() {
        let actions = resolve(&[diag(NumberOrString::String("E5060".into()))]);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CodeActionOrCommand::CodeAction(a) => {
                assert_eq!(a.title, "Explain E5060");
                let cmd = a.command.as_ref().unwrap();
                assert_eq!(cmd.command, "mcode.explain");
                assert_eq!(cmd.arguments.as_ref().unwrap()[0], serde_json::json!("E5060"));
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn plain_numbers_and_bare_digits_both_parse() {
        let actions = resolve(&[
            diag(NumberOrString::Number(5060)),
            diag(NumberOrString::String("5060".into())),
        ]);
        // Same code in two spellings dedups to one action.
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn non_numeric_codes_yield_nothing() {
        let actions = resolve(&[diag(NumberOrString::String("not-a-code".into()))]);
        assert!(actions.is_empty());
        let actions = resolve(&[]);
        assert!(actions.is_empty());
    }

    #[test]
    fn fix_diagnostic_yields_a_workspace_edit_action() {
        let actions = resolve(&[fix_diag("vout", 5)]);
        // One Explain (the numeric code) + one rename.
        assert_eq!(actions.len(), 2);
        match &actions[1] {
            CodeActionOrCommand::CodeAction(a) => {
                assert_eq!(a.title, "Rename 'vout' to 'VOUT'");
                // 1-based wire positions land as 0-based LSP positions.
                let edit = a.edit.as_ref().unwrap();
                let changes = edit.changes.as_ref().unwrap();
                let edits = changes.values().next().unwrap();
                assert_eq!(edits.len(), 1);
                assert_eq!(edits[0].range.start, Position::new(2, 4));
                assert_eq!(edits[0].range.end, Position::new(2, 8));
                assert_eq!(edits[0].new_text, "VOUT");
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn two_fixes_merge_into_one_fix_all() {
        let actions = resolve(&[fix_diag("vout", 5), fix_diag("gnd", 30)]);
        // One Explain (the shared code) + two rename actions + one fix-all.
        assert_eq!(actions.len(), 4);
        match &actions[3] {
            CodeActionOrCommand::CodeAction(a) => {
                assert_eq!(a.title, "Fix all style names in file");
                let edit = a.edit.as_ref().unwrap();
                let edits = edit.changes.as_ref().unwrap().values().next().unwrap();
                assert_eq!(edits.len(), 2);
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn identical_edits_dedup_in_fix_all() {
        // Two diagnostics flagging the same name carry overlapping edit sets;
        // the fix-all must apply each occurrence once.
        let actions = resolve(&[fix_diag("vout", 5), fix_diag("vout", 5)]);
        match &actions[3] {
            CodeActionOrCommand::CodeAction(a) => {
                let edits = a
                    .edit
                    .as_ref()
                    .unwrap()
                    .changes
                    .as_ref()
                    .unwrap()
                    .values()
                    .next()
                    .unwrap();
                assert_eq!(edits.len(), 1);
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn plain_diagnostic_carries_no_rename_action() {
        let actions = resolve(&[diag(NumberOrString::Number(5060))]);
        assert!(actions.iter().all(|a| match a {
            CodeActionOrCommand::CodeAction(c) => c.edit.is_none(),
            _ => false,
        }));
    }
}
