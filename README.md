# MCode Language Server

A VSCode language extension providing language server features for MCode files (`.mc`).

## Features

The capability list lives in one place: **[doc/features.md](doc/features.md)**
— the feature matrix, one row per capability, with its LSP entry point, its
implementation status and a link to the capability's own document. That
matrix is the only implementation truth in this repository; this README does
not restate it.

What is in place today: semantic highlighting (full / delta / range),
auto-completion, go to definition, find references, diagnostics, hover,
rename, document outline, formatting, folding, selection range, document
highlight, code actions, workspace symbol search, snippets, and the
infrastructure around them (settings, commands, keybindings, status bar,
netlist preview).

- **Dependency readiness**: building a project checks the libraries its
  manifest declares and installs the ones that are missing before the build
  runs (see [Dependencies](#dependencies))
- **Negotiated with the kernel**: on connect the extension hands the mcc
  daemon its version triple and reads the daemon's capability sheet back.
  The verdict decides whether the daemon may be used; the sheet says what it
  can answer to. The sheet is visible in the status bar tooltip and in the
  `MCode Build` output channel.

## Dependencies

A project declares its libraries in `project.toml`:

```toml
[dependencies]
mcfam = "0.141"
mcaps = "*"
```

- A bare version (`"0.141"`) is an **exact pin**: that exact version, not
  "any 0.141.x". `*` states no requirement — any installed copy serves.
- The operators the version law knows (`^`, `~`, `>=`, `<`) state a range;
  a range is resolved against the registry.

**MCode: Build Project** asks the daemon whether each declared library is
in place. A library that is not gets solved and installed (the daemon's
`lib.resolve` — the same face the CLI's `mcc lib resolve` is), and the
build then runs against it. The build report names what was installed and
what is still missing.

Installing needs a registry: set `[registry] url` in the global `mcc.yaml`
(or `MCC_REGISTRY`), or `[config.registry] url` in the project's
`project.toml`. Without one, the build reports the missing libraries and
continues — a project whose libraries are already installed builds with no
registry at all.

## Requirements

- VSCode 1.66.0 or higher
- Rust toolchain (for building the language server)

## Project Structure

- `client/` - VSCode extension client (TypeScript)
- `src/` - Language server implementation (Rust)
- `syntaxes/` - TextMate grammar definitions
- `data/` - Sample MCode files for testing

## Development

```bash
# Install dependencies
pnpm install
cd client && pnpm install

# Build
pnpm run compile

# Watch mode
pnpm run watch

# Run extension
# Make sure you select "Debug VSCode Extension" in VSCode's Run and Debug window (Ctrl + Shift + D)
F5 (in VSCode with Rust Analyzer)
```

## License

MIT
