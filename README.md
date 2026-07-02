# MCode Language Server

A VSCode language extension providing language server features for MCode files (`.mc`).

## Features

- **Semantic Highlighting**: Syntax highlighting based on semantic token types
- **Auto-completion**: Context-aware completion suggestions
- **Go to Definition**: Navigate to symbol definitions
- **Find References**: Locate all references to a symbol
- **Diagnostics**: Real-time error and warning detection
- **Inlay Hints**: Type annotations and parameter hints
- **Formatting**: Code formatting support
- **Cross-file Support**: Full language server features across multiple files

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
