//! Use-path completion — segment-wise candidates for the four `use` forms
//! (completion-design §5.7; corpus evidence §12 #14: `use ./power.mc`,
//! `use ./ifs/xtal`, `use $::mcpub.power/ams1117.mc`, `use mclibs.power.tle7368`).
//!
//! Resolution semantics mirror mcc's `McUse` parsing
//! (`mcc/src/db/infra/mc_use.rs`) and the grammar (`mcast/src/mca.y`): the
//! bare unprefixed form is synthesized with a `$` (system) prefix, single
//! module segments auto-double (`conn` → `conn/conn.mc`), `@version` sits
//! before the implicit `.mc` suffix, and `as` aliases have nothing to
//! complete. All candidates are resolved from disk against the same roots
//! `util::usechk` resolves for goto-def — no mcc round trip.

use std::path::{Path, PathBuf};

use ropey::Rope;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionTextEdit, Position, Range, TextEdit, Url,
};

use crate::common::position::offset_to_position;
use crate::project::find_project_root_from_file;
use crate::util::usechk;

/// The four use-form prefixes (grammar `mc_prefix` + the synthesized bare/system form).
const FORM_PREFIXES: [&str; 4] = ["./", "../", "/", "$::"];

/// One candidate: the full replacement text (a `TextEdit` spans the partial
/// path already typed), its item kind, and a short human hint.
struct Candidate {
    full: String,
    kind: CompletionItemKind,
    detail: String,
}

/// Completions for a `use` / `pub use` line at the cursor byte offset.
///
/// Returns `None` when the cursor is not inside the path token (still on the
/// keyword, or past it in `as`/`import(...)` territory) — the caller then
/// falls through to keyword completion.
pub fn completions(
    rope: &Rope,
    uri: &Url,
    offset: usize,
    cursor: Position,
) -> Option<Vec<CompletionItem>> {
    completions_with(&usechk::system_roots(), rope, uri, offset, cursor)
}

/// `completions` with the system-library roots injected (tests pin them to a
/// fixture directory so results never depend on machine state).
pub fn completions_with(
    roots: &[PathBuf],
    rope: &Rope,
    uri: &Url,
    offset: usize,
    cursor: Position,
) -> Option<Vec<CompletionItem>> {
    // ropey's line index is 0-based (note: common::position::offset_to_line
    // is the mcc-facing 1-based variant and must not be used here).
    let line_idx = rope.byte_to_line(offset);
    let line_start = rope.line_to_byte(line_idx);
    let line_before = rope.byte_slice(line_start..offset).to_string();

    let trimmed = line_before.trim_start();
    let (kw_len, after_kw) = if let Some(r) = trimmed.strip_prefix("pub use") {
        (7, r)
    } else if let Some(r) = trimmed.strip_prefix("use") {
        (3, r)
    } else {
        return None;
    };
    // The path token starts after one run of whitespace; while that run is
    // still being typed the keyword itself is what's completing.
    let ws_len = after_kw.len() - after_kw.trim_start().len();
    if after_kw.trim_start().is_empty() && ws_len == 0 {
        return None;
    }
    // Slice the partial path out of the line text (indent + keyword + whitespace).
    let indent = line_before.len() - trimmed.len();
    let path_start = line_start + indent + kw_len + ws_len;
    let typed = line_before[indent + kw_len + ws_len..].to_string();
    if typed.contains(char::is_whitespace) {
        // Past the path token (`as tgt`, `import(...)`) — no candidates.
        return None;
    }

    let mut cands: Vec<Candidate> = Vec::new();
    if typed.is_empty() || FORM_PREFIXES.iter().any(|f| f.starts_with(&typed)) {
        // Still typing a form prefix (or nothing at all): offer the forms.
        // The bare (unprefixed) form is the system form per the grammar, so
        // plain `conn` completes via the system walk below instead.
        for f in FORM_PREFIXES {
            if f.starts_with(&typed) {
                cands.push(Candidate {
                    full: f.to_string(),
                    kind: CompletionItemKind::KEYWORD,
                    detail: form_hint(f).to_string(),
                });
            }
        }
        return Some(extend(cands, path_start, cursor, rope));
    }

    // Whatever the form prefix was keeps its exact typed spelling — the
    // TextEdit replaces the whole partial path, so candidates must
    // reassemble it.
    if let Some(rest) = typed.strip_prefix("./") {
        let dir = uri.to_file_path().ok()?.parent()?.to_path_buf();
        cands.extend(relative_walk(&dir, "./", rest, '/'));
    } else if let Some(rest) = typed.strip_prefix("../") {
        let dir = uri.to_file_path().ok()?.parent()?.parent()?.to_path_buf();
        cands.extend(relative_walk(&dir, "../", rest, '/'));
    } else if let Some(rest) = typed.strip_prefix('/') {
        let file = uri.to_file_path().ok()?;
        let root = find_project_root_from_file(&file)?;
        cands.extend(relative_walk(&root, "/", rest, '/'));
    } else {
        // `$`-prefixed or bare: the system-library space, over every root
        // `usechk` resolves for goto-def. Module segments separate on `.`
        // (corpus `mclibs.power.tle7368`) or `/` (`mcpub.power/ams1117.mc`)
        // — whichever the typist started wins.
        let bare = typed.strip_prefix('$').unwrap_or(&typed);
        let rest = bare.strip_prefix("::").unwrap_or(bare);
        let sep = if rest.contains('/') { '/' } else { '.' };
        let form = &typed[..typed.len() - rest.len()];
        for root in roots {
            cands.extend(relative_walk(root, form, rest, sep));
        }
        cands.sort_by(|a, b| a.full.cmp(&b.full));
        cands.dedup_by(|a, b| a.full == b.full);
    }

    Some(extend(cands, path_start, cursor, rope))
}

fn form_hint(prefix: &str) -> &'static str {
    match prefix {
        "./" => "current directory",
        "../" => "parent directory",
        "/" => "project root",
        "$::" => "system library",
        _ => "",
    }
}

/// Attach the shared `TextEdit` (spanning the partial path) to each candidate.
fn extend(cands: Vec<Candidate>, path_start: usize, cursor: Position, rope: &Rope) -> Vec<CompletionItem> {
    let start = offset_to_position(path_start, rope).unwrap_or(cursor);
    cands.into_iter()
        .map(|c| {
            let mut item = CompletionItem::new_simple(c.full.clone(), c.detail);
            item.kind = Some(c.kind);
            item.text_edit = Some(CompletionTextEdit::Edit(TextEdit {
                range: Range::new(start, cursor),
                new_text: c.full,
            }));
            item
        })
        .collect()
}

/// Walk `base` for `rest` (segments split and reassembled on `sep`): every
/// segment but the last must be an existing directory; the last segment
/// filters entries. Files are offered with their `.mc` suffix, directories
/// with a trailing separator (the corpus module form `./ifs/xtal`).
fn relative_walk(base: &Path, form: &str, rest: &str, sep: char) -> Vec<Candidate> {
    let (dir, filter, typed_prefix) = walk_to(base, form, rest, sep);
    let mut cands = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return cands;
    };
    let mut names: Vec<(String, bool)> = entries
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (name, is_dir)
        })
        .collect();
    names.sort();
    for (name, is_dir) in names {
        let full = if is_dir {
            if !name.starts_with(&filter) {
                continue;
            }
            format!("{typed_prefix}{name}{sep}")
        } else {
            if !name.ends_with(".mc") || !name.starts_with(&filter) {
                continue;
            }
            format!("{typed_prefix}{name}")
        };
        cands.push(Candidate {
            full,
            kind: if is_dir {
                CompletionItemKind::FOLDER
            } else {
                CompletionItemKind::FILE
            },
            detail: if is_dir {
                "directory".to_string()
            } else {
                "module file".to_string()
            },
        });
    }
    if filter.contains('@') {
        cands.extend(version_candidates(&dir, &filter, &typed_prefix));
    }
    cands
}

/// Resolve the walk directory, the last-segment filter, and the typed text
/// the candidates must reassemble (every segment but the last plus its
/// separator).
fn walk_to(base: &Path, form: &str, rest: &str, sep: char) -> (PathBuf, String, String) {
    let mut dir = base.to_path_buf();
    let segs: Vec<&str> = rest.split(sep).collect();
    let (segs, filter) = segs.split_at(segs.len() - 1);
    for seg in segs {
        dir.push(seg);
    }
    let joined = segs.join(&sep.to_string());
    let typed_prefix = if joined.is_empty() {
        form.to_string()
    } else {
        format!("{form}{joined}{sep}")
    };

    (dir, filter.first().unwrap_or(&"").to_string(), typed_prefix)
}

/// `@version` candidates: files named `<stem>@<version>.mc` next to the
/// module path (mcc joins `uri@version` + `.mc` in `update_abs_path`), with
/// the stem taken from before the `@` and any partial version after it
/// filtering the extracted versions.
fn version_candidates(dir: &Path, filter: &str, typed_prefix: &str) -> Vec<Candidate> {
    let (stem, ver_filter) = match filter.split_once('@') {
        Some((stem, ver)) => (stem.trim_end_matches(".mc"), Some(ver)),
        None => return Vec::new(),
    };
    let prefix = format!("{stem}@");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut vers: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(&prefix)?;
            let ver = rest.strip_suffix(".mc")?;
            if ver.is_empty() {
                return None;
            }
            Some(ver.to_string())
        })
        .filter(|v| ver_filter.map_or(true, |f| v.starts_with(f)))
        .collect();
    vers.sort();
    vers.dedup();
    vers.into_iter()
        .map(|v| Candidate {
            full: format!("{typed_prefix}{stem}@{v}"),
            kind: CompletionItemKind::VALUE,
            detail: "library version".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::position::position_to_offset;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn fixture(files: &[&str]) -> PathBuf {
        let n = FIXTURE_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("u258-usepath-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "// fixture\n").unwrap();
        }
        dir
    }

    fn completions_in(dir: &Path, text: &str) -> Option<Vec<CompletionItem>> {
        let file = dir.join("main.mc");
        std::fs::write(&file, text).unwrap();
        let uri = Url::from_file_path(&file).unwrap();
        let rope = Rope::from_str(text);
        let col = text.chars().count() as u32;
        let cursor = Position::new(0, col);
        let offset = position_to_offset(cursor, &rope).unwrap();
        completions_with(&[dir.to_path_buf()], &rope, &uri, offset, cursor)
    }

    fn labels(items: &[CompletionItem]) -> Vec<String> {
        items.iter().map(|i| i.label.clone()).collect()
    }

    #[test]
    fn empty_token_offers_the_four_forms() {
        let dir = fixture(&["power.mc"]);
        let items = completions_in(&dir, "use ").expect("form items");
        assert_eq!(labels(&items), vec!["./", "../", "/", "$::"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_dot_offers_dot_forms_only() {
        let dir = fixture(&["power.mc"]);
        let items = completions_in(&dir, "use .").expect("form items");
        assert_eq!(labels(&items), vec!["./", "../"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_file_and_directory_candidates() {
        let dir = fixture(&["power.mc", "ifs/xtal.mc"]);
        // `use ./p` → the file.
        let items = completions_in(&dir, "use ./p").expect("file candidate");
        assert_eq!(labels(&items), vec!["./power.mc"]);
        // `use ./if` → the directory with a trailing separator.
        let items = completions_in(&dir, "use ./if").expect("dir candidate");
        assert_eq!(labels(&items), vec!["./ifs/"]);
        // Inside the directory the entry file completes too.
        let items = completions_in(&dir, "use ./ifs/").expect("dir walk");
        assert_eq!(labels(&items), vec!["./ifs/xtal.mc"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bare_token_walks_the_system_roots_dotted() {
        let dir = fixture(&["man/man.mc"]);
        let items = completions_in(&dir, "use man").expect("system candidates");
        // No separator typed yet → module segments reassemble with `.`.
        assert_eq!(labels(&items), vec!["man."]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dollar_prefixes_and_both_separators_walk_the_same_roots() {
        let dir = fixture(&["mcpub/power/ams1117.mc"]);
        // `$::`, `$`, and the bare form all reach the system space; each next
        // module segment completes one at a time (§5.7 按段逐段补全).
        for typed in ["$::mcpub", "$mcpub", "mcpub"] {
            let items = completions_in(&dir, &format!("use {typed}"))
                .unwrap_or_else(|| panic!("no items for {typed:?}"));
            assert_eq!(labels(&items), vec![format!("{typed}.")]);
        }
        // A typed `/` keeps reassembling with `/` (corpus `mcpub.power/ams1117.mc`
        // mixes them), and the entry file completes inside the last directory.
        let items = completions_in(&dir, "use $::mcpub/power/a").expect("file deep in lib");
        assert_eq!(labels(&items), vec!["$::mcpub/power/ams1117.mc"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_candidates_after_the_at_sign() {
        let dir = fixture(&["power@2.1.0.mc", "power@2.2.0.mc", "power.mc"]);
        let items = completions_in(&dir, "use ./power@2.").expect("version candidates");
        // The explicit `<stem>@<ver>.mc` files are also offered verbatim by
        // the walk; the version stage adds the bare `@version` sugar.
        let labs = labels(&items);
        assert!(labs.contains(&"./power@2.1.0".to_string()), "{labs:?}");
        assert!(labs.contains(&"./power@2.2.0".to_string()), "{labs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn alias_stage_yields_nothing() {
        let dir = fixture(&["power.mc"]);
        assert_eq!(completions_in(&dir, "use ./power.mc as "), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keyword_stage_yields_nothing() {
        let dir = fixture(&["power.mc"]);
        assert_eq!(completions_in(&dir, "use"), None);
        assert_eq!(completions_in(&dir, "pub use"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_span_the_partial_path() {
        let dir = fixture(&["power.mc"]);
        let items = completions_in(&dir, "use ./p").expect("items");
        let edit = match items[0].text_edit.as_ref().expect("text edit") {
            CompletionTextEdit::Edit(edit) => edit,
            other => panic!("unexpected text edit shape: {other:?}"),
        };
        assert_eq!(edit.range.start, Position::new(0, 4));
        assert_eq!(edit.range.end, Position::new(0, 7));
        assert_eq!(edit.new_text, "./power.mc");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
