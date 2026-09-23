//! Code actions — quick-fix surface sourced from the diagnostics already in
//! the request context. mcc's `suggestions` are free-text hints that reference
//! locations, not applicable edits, so no `WorkspaceEdit` is fabricated; the
//! one concrete action is *explain*: run the numeric diagnostic code through
//! the mcc `explain` RPC (rule owner, acceptance, fix hints, allow-syntax).

use tower_lsp::lsp_types::{CodeAction, CodeActionKind, CodeActionOrCommand, Command, Diagnostic};

/// Actions for one `textDocument/codeAction` request: one explain action per
/// distinct numeric code in the context's diagnostics. Returns an empty vec
/// (not an error) when nothing is explainable — the editor renders no menu.
pub fn resolve(diagnostics: &[Diagnostic]) -> Vec<CodeActionOrCommand> {
    let mut actions: Vec<CodeActionOrCommand> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for diag in diagnostics {
        let Some(code) = numeric_code(diag) else {
            continue;
        };
        if seen.contains(&code) {
            continue;
        }
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
    actions
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
}
