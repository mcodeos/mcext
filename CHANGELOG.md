# Changelog

All notable changes to the MCode VS Code extension are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.5.2] - 2026-09-24

### Added

- **Hover drill-down**: hovering a component / module / interface / enum —
  its definition or a reference resolved to it — now appends the mcc `show.*`
  card (pin table, interfaces, description) below the local definition hover.
  A busy or unreachable server keeps the local hover untouched.
- **Keybindings** on `.mc` files: `ctrl+alt+b` build project, `ctrl+alt+c`
  check current file, `ctrl+alt+e` explain error code, `ctrl+alt+v` preview
  circuit.
- **Semantic-token theme scopes** for the `mcode` language, so mcc's token
  legend renders with the editor theme's colors even without a dedicated
  theme.
- **Status bar item** showing the language-server lifecycle (starting /
  ready / failed); clicking it runs a project build.

### Changed

- `mcodels.diagnosticsDebounceMs` is now applied live: the reparse scheduler
  picks up configuration changes (and startup values) without a restart.
- Completion triggers extended with `/` (use-path segments) and `@` (library
  versions).

### Removed

- Rust test fixtures are no longer packaged into the `.vsix`.

## [0.5.1] - 2026-09-24

### Added

- **Cross-file rename**: `textDocument/rename` now drives off the mcc `refs`
  RPC (whole workspace) with a per-row safety gate — every span is verified to
  contain exactly the word under the cursor before an edit is produced. Falls
  back to the local current-file path when the server is unreachable.
- **Error-code explain** (`MCode: Explain Error Code`): prompt for a code
  (`E5060` / `5060`), answered by the mcc `explain` RPC.
- **Check current file** (`MCode: Check Current File`): mcc `check` dry-run on
  the active `.mc` file; the summary counts (file + library errors/warnings)
  are shown without touching the editor buffer.
- **Completion detail layers** (S5): attribute-key completion at
  `ident =` positions — known keys plus names assigned in the current file —
  and `completionItem/resolve` grounding that attaches component / module /
  interface / enum documentation from the mcc `show.*` RPCs.
- **More declared capabilities**: code action (explain quick fixes), folding
  ranges, document highlight, selection ranges, prepare-rename, workspace
  symbol, `document_symbol` and `rename` declarations.
- **Settings**: `mcodels.systemRoot`, `mcodels.projectRoot`,
  `mcodels.semanticTokensEnabled`, `mcodels.inlayHintsEnabled`,
  `mcodels.diagnosticsDebounceMs`, `mcodels.formatTabSize`,
  `mcodels.formatInsertFinalNewline` — forwarded to the server at startup and
  merged live on `workspace/didChangeConfiguration` (format tab size and the
  semantic-token / inlay-hint gates take effect; the debounce value is stored
  but not yet applied to the reparse scheduler).

### Changed

- The startup handshake probes the mcc `caps` RPC first (schema version +
  method surface) and falls back to `server.info` for older binaries.

### Removed

- Dead `activateInlayHints` scaffolding from the client.
