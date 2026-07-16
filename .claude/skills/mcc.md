# mcc Compiler

> CLI reference, RPC protocol, and debugging workflows for the mcc compiler and mcode projects.

---

## 1. Quick Reference

### Build & Run

```bash
cd /Users/dan/work/mo/mcc
cargo build
```

### Key Paths

| Path | Purpose |
|---|---|
| `/Users/dan/work/mo/mcc` | Compiler source |
| `/Users/dan/work/mo/mcode` | Standard library (components, interfaces, packages) |
| `/Users/dan/work/mo/mcd` | Workspace: test projects, libraries, docs |
| `/Users/dan/work/mo/mcext` | VS Code extension + LSP server (`mcodels`) |
| `~/.mcode/` | Runtime data: config, logs, PID file |
| `~/.mcode/config/mcc.yaml` | Global compiler config |
| `~/.mcode/config/server.yaml` | RPC server config |
| `~/.mcode/logs/mcc.pid` | Server PID file |

### Environment Variables

| Variable | Purpose |
|---|---|
| `MCC_SYSTEM_ROOT` | Override data directory (default `~/.mcode`) |
| `RUST_LOG` | Tracing filter (overrides `-v`/`-q`) |
| `MCC_LOG_FILE` | Redirect C-parser trace to file |
| `MCC_GOLDEN_PROJECT` | Golden-test project root |
| `MCC_GOLDEN_ENTRY` | Golden-test entry file |
| `MCC_GOLDEN_TOP` | Golden-test top module |
| `UPDATE_GOLDEN` | Write golden baseline instead of comparing |
| `MC_VIZ_DUMP` | Enable visualization debug dump |

---

## 2. CLI Commands

### Global Flags

```
-v, -vv, -vvv     Verbose (info / debug / trace)
-q                Quiet (errors only)
-t                Show module/file in log lines
--cwd DIR          Working directory
--completion SHELL Generate shell completions (bash/zsh/fish/powershell)
-V, --version     Print version
```

### Legacy Shorthand

```bash
mcc example.mc main --viz
# Auto-rewritten to: mcc parse example.mc --top main --viz
```

---

### 2.1 `parse` — Parse & Analyze

```bash
# Parse a single file
mcc parse path/to/file.mc

# Parse a project directory (auto-detects project.toml)
mcc parse ./my-project

# Parse a code snippet directly
mcc parse --code "RES(100Ω, 250V)" --lib mcode

# Parse with top module and visualization
mcc parse example.mc --top main --viz

# Parse-only, no instantiation
mcc parse example.mc --pass1

# Output as JSON
mcc parse example.mc -f json-pretty -o result.json

# Show AST
mcc parse example.mc --ast

# Limit tree depth
mcc parse example.mc --top main --depth 3
```

Key flags:
| Flag | Purpose |
|---|---|
| `--code CODE` | Parse inline code |
| `--lib NAME` | Load system library (repeatable) |
| `--top NAME` | Top-level module name |
| `--sort {pinid\|interface}` | Pin sorting mode |
| `--pass1` | Parse only (no instantiation) |
| `--pass2` | Parse + instantiate |
| `--viz` | Generate HTML visualization |
| `--viz-json` | Generate JSON visualization data |
| `--ast` | Print AST |
| `--tree` | Print tree representation |
| `--depth N` | Tree depth limit (0=unlimited) |
| `-f FORMAT` | text, json, json-pretty, yaml, csv |
| `-o FILE` | Output file |

---

### 2.2 `check` — Validate

```bash
# Check a file and print diagnostics
mcc check path/to/file.mc

# Check entire project directory
mcc check ./my-project

# Errors only
mcc check example.mc --errors-only

# Strict mode (warnings become errors)
mcc check example.mc --strict

# Include netlist checks
mcc check example.mc --nets

# JSON output
mcc check example.mc -f json-pretty
```

---

### 2.3 `build` — Manifest-Driven Build

```bash
# Build from project.toml in current directory
mcc build

# Build with explicit entry file
mcc build path/to/main.mc

# Override top module
mcc build --top my_top_module

# With visualization
mcc build --viz

# Include system library in output
mcc build --include-system

# Lock specific layouter
mcc build --viz --layouter schematic_flow
```

Uses `project.toml` / `manifest.toml` / `mcc.toml`:
```toml
[project]
name = "hbl"
version = "0.1.0"
entry = "src/hbl.mc"
top_module = "main"

[dependencies]
mcode = "*"
```

---

### 2.4 `show` — Inspect Definitions

```bash
# List all components
mcc show component

# Show component details
mcc show component --name RES

# List all interfaces
mcc show interface

# Show pins of a component
mcc show pins --name RES

# Show instances in a module
mcc show instances -F example.mc -T main

# Show all entities in a file
mcc show file -F example.mc

# Filter results
mcc show component --filter "name=RES*"

# Dump everything (debug)
mcc show dump.all
```

Show targets: `all`, `file`, `files`, `component`, `module`, `interface`, `enum`, `net`, `pins`, `ports`, `labels`, `instances`, `nets`, `attrs`, `funcs`, `params`, `roles`, `values`, `dump`

---

### 2.5 `search` & `query` — Find Definitions

```bash
# Text search
mcc search RES

# Regex search
mcc search "CAP\..*" --regex

# Fuzzy search
mcc search "amplifir" --fuzzy

# Filter by kind
mcc search SPI --kind interface

# Limit results
mcc search RES --limit 10

# JSON output
mcc search RES --json
```

```bash
# Structured DSL query
mcc query "kind=component AND name=RES*"

# Query with filters
mcc query "kind=interface AND port_count>2" --json
```

---

### 2.6 `export` — Generate Outputs

```bash
# Netlist
mcc export netlist example.mc --top main

# BOM (Bill of Materials)
mcc export bom example.mc --top main

# SPICE netlist
mcc export spice example.mc --top main

# KiCad schematic
mcc export kicad example.mc --top main -o output.kicad_sch

# JSON format
mcc export netlist example.mc --top main --json
```

---

### 2.7 `extract` — Extract Entities

```bash
# All instances
mcc extract instances example.mc --top main

# All nets
mcc extract nets example.mc --top main

# Components only
mcc extract components example.mc

# Interfaces only
mcc extract interfaces example.mc

# Filter by name pattern
mcc extract instances example.mc --name "C*"
```

---

### 2.8 `lib` — Library Management

```bash
# List loaded libraries
mcc lib list

# Show library info
mcc lib show mcode

# Install a library from source
mcc lib install /path/to/library

# Search available libraries
mcc lib search mcode

# Uninstall
mcc lib uninstall mylib

# Load/unload at runtime
mcc lib load mylib
mcc lib unload mylib
```

---

### 2.9 `start` / `stop` / `status` — RPC Server

```bash
# Start foreground server
mcc start --host 127.0.0.1 --port 8080 --lib mcode

# Start background daemon
mcc start -d --port 8080 --lib mcode

# With logging
mcc start --log-level debug -l /tmp/mcc-server.log

# Check status
mcc status
mcc status --json

# Stop gracefully
mcc stop

# Force stop
mcc stop -f
```

---

### 2.10 Other Commands

```bash
# Create a new project
mcc proj create my-project

# Explain an error code
mcc explain 1100

# Go-to-definition
mcc def RES --lib mcode

# Find references
mcc refs RES --lib mcode

# Electrical rule check
mcc erc ./my-project --top main

# Convert .mc to JSON
mcc convert example.mc --to json -o example.json

# Generate design report
mcc report ./my-project

# Self-describing capabilities (AI discovery)
mcc caps

# Config management
mcc config list
mcc config get parser.sort_pins
mcc config set trace.pass1 true
mcc config reset trace.pass1
```

---

## 3. RPC Protocol

### Overview

JSON-RPC 2.0 over HTTP. Server listens on `127.0.0.1:{port}` (default 8080).

| Endpoint | Method | Purpose |
|---|---|---|
| `/rpc` | POST | Main JSON-RPC handler |
| `/health` | POST | Health check → `{"status": "ok"}` |

Request format:
```json
{"jsonrpc": "2.0", "method": "server.info", "params": {}, "id": 1}
```

Response format:
```json
{"jsonrpc": "2.0", "result": {...}, "id": 1}
```

Error format:
```json
{"jsonrpc": "2.0", "error": {"code": -32601, "message": "Method not found"}, "id": 1}
```

### Client Usage (curl)

```bash
# Health check
curl -X POST http://127.0.0.1:8080/health

# Server info
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"server.info","params":{},"id":1}'

# List methods
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"server.methods","params":{},"id":2}'

# Parse a file
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"parse","params":{"uri":"file:///path/to/file.mc"},"id":3}'

# Show all components
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"show.component","params":{},"id":4}'

# Get diagnostics for a file
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"diagnostics","params":{"uri":"file:///path/to/file.mc"},"id":5}'
```

### RPC Methods Reference

#### Discovery
| Method | Params | Returns |
|---|---|---|
| `server.info` | — | Server version, uptime, loaded libs |
| `server.methods` | — | List of all registered methods |
| `caps` | — | Self-describing capabilities |

#### Workspace
| Method | Params | Returns |
|---|---|---|
| `init` | — | Initialize workspace |
| `load_project` | `uri` | Load project entry file |
| `add_file` | `uri` | Add file to workspace |
| `remove_file` | `uri` | Remove file from workspace |
| `set_project_root` | `path` | Set project root directory |
| `set_system_root` | `path` | Set system library root |

#### Parse / Build
| Method | Params | Returns |
|---|---|---|
| `parse` | `uri`, `code?`, `libs?` | Parse result |
| `check` | `uri` | Diagnostics |
| `build.full` | `uri`, `top?` | Full build result |
| `extract` | `kind`, `uri`, `top?`, `name?` | Extracted entities |

#### Show / Inspect
| Method | Params | Returns |
|---|---|---|
| `show.component` | `name?` | Component list or detail |
| `show.module` | `name?` | Module list or detail |
| `show.interface` | `name?` | Interface list or detail |
| `show.enum` | `name?` | Enum list or detail |
| `show.net` | `name?` | Net list or detail |
| `show.pins` | `name` | Pin definitions |
| `show.ports` | `name` | Port definitions |
| `show.instances` | `file`, `top` | Instance list |
| `show.nets` | `file`, `top` | Net list |
| `show.attrs` | `name` | Attribute list |
| `show.funcs` | `name` | Function list |
| `show.params` | `name` | Parameter list |
| `show.roles` | `name` | Role definitions |
| `show.dump` | `name` | Full entity dump |
| `show.file` | `uri` | All definitions in file |
| `show.files` | — | All loaded files |

#### Semantics / LSP
| Method | Params | Returns |
|---|---|---|
| `sem` | `uri`, `content?` | Semantic tokens + symbols |
| `diagnostics` | `uri` | File diagnostics |
| `project_symbols` | — | Project-wide symbol index |
| `def` | `name`, `uri?` | Go-to-definition |
| `refs` | `name`, `uri?` | Find all references |
| `erc` | `uri?`, `top?` | Electrical rule check |

#### Library
| Method | Params | Returns |
|---|---|---|
| `lib.list` | — | Loaded libraries |
| `lib.info` | `name` | Library metadata |
| `lib.load` | `name` | Load a library |
| `lib.unload` | `name` | Unload a library |
| `lib.install` | `path` | Install library |
| `lib.uninstall` | `name` | Uninstall library |
| `lib.search` | `query` | Search installed libs |

#### Export / Utility
| Method | Params | Returns |
|---|---|---|
| `export` | `kind`, `uri`, `top` | Export result |
| `convert` | `uri`, `format` | Convert file |
| `report` | `uri?` | Design report |
| `explain` | `code?` | Error code description |
| `trace.set` | `config` | Update trace config |
| `trace.get` | — | Current trace config |

### Error Codes

| Code | Meaning |
|---|---|
| -32700 | Parse error (invalid JSON) |
| -32600 | Invalid request |
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error |
| 32100 | I/O or filesystem error |
| 32101 | Workspace conflict |
| 32102 | Workspace not found |
| 32103 | Archive decode failed |
| 32104 | Unsupported format |
| 32105 | Entry file not found |
| 32106 | Dependency not loaded |
| 32107 | Pass1 or Pass2 failed |
| 32108 | Build panic |

---

## 4. Compiler Pipeline

```
Pass 0 — Manifest
  Read project.toml → load dependencies → resolve entry file

Pass 1 — Parse
  C lexer + yacc parser → AST → type resolution → cross-file references
  Output: definitions by URI + span

Pass 2 — Instantiate
  Top module → recursive instantiation → McProjectTree + InstTable
  Output: ports, components, submodules, connections, nets

Pass 3 — Vector
  build_mc_vec → McVecBlock → build_mc_vec_graph → McVecGraph
  D1-D8 detectors run here (codes 2001-2008)

Pass 4 — Layout + Render
  Layout algorithms → wire routing → SVG render → HTML template
```

```bash
# Run specific passes
mcc parse example.mc --pass1              # Pass 1 only
mcc parse example.mc --pass2 --top main   # Pass 1 + 2
mcc parse example.mc --viz --top main     # All passes (visualization)

# Trace pass execution (via RPC)
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"trace.set","params":{"pass1":true,"pass2":true},"id":1}'
```

---

## 5. Debugging mcc Itself

### 5.1 VS Code Debug Configurations

In `/Users/dan/work/mo/mcc/.vscode/launch.json`:

**"mcc"** — Debug a one-shot CLI run:
- Program: `target/debug/mcc`
- Args: `mc/projects/hbl/hbl.mc`
- Env: `RUST_BACKTRACE=1`, `MCC_SYSTEM_ROOT=${workspaceFolder}/mc`
- cwd: `${workspaceFolder}`

```bash
# Equivalent command line
cd /Users/dan/work/mo/mcc
MCC_SYSTEM_ROOT=./mc RUST_BACKTRACE=1 cargo run -- mc/projects/hbl/hbl.mc
```

### 5.2 Logging

```bash
# Increasing verbosity
mcc parse example.mc               # warnings only
mcc parse example.mc -v            # info
mcc parse example.mc -vv           # debug
mcc parse example.mc -vvv          # trace (very verbose)

# Target-specific logging
RUST_LOG="mcc::pass1=trace,mcc::pass2=debug" mcc parse example.mc

# With target names shown
mcc parse example.mc -vvv -t

# C parser trace
MCC_LOG_FILE=/tmp/cparse.log mcc parse example.mc -vvv

# Visualization debug dump
MC_VIZ_DUMP=1 mcc parse example.mc --viz --top main
```

### 5.3 Server Debugging

```bash
# Foreground server with full tracing
mcc start --port 8080 --log-level debug --lib mcode

# Check server health
curl -X POST http://127.0.0.1:8080/health

# Check if server is running
mcc status

# View PID
cat ~/.mcode/logs/mcc.pid

# Kill orphaned server
kill $(cat ~/.mcode/logs/mcc.pid)

# Force stop if hung
mcc stop -f
```

### 5.4 Trace Configuration (runtime)

```bash
# Enable pass1 tracing via RPC
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"trace.set","params":{"enabled":true,"pass1":true},"id":1}'

# Check current trace config
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"trace.get","params":{},"id":1}'
```

### 5.5 Test Commands

```bash
# Run all tests
cd /Users/dan/work/mo/mcc
cargo test

# Run specific test
cargo test --lib cmds::build::tests

# Run golden tests
MCC_GOLDEN_PROJECT=/path/to/project cargo test golden

# Update golden baselines
UPDATE_GOLDEN=1 MCC_GOLDEN_PROJECT=/path/to/project cargo test golden

# Run with backtrace
RUST_BACKTRACE=full cargo test
```

---

## 6. Debugging mcode Projects

### 6.1 Project Structure

```
my-project/
├── project.toml          # Required: [project] + [dependencies]
├── src/
│   ├── main.mc           # Entry file (referenced in project.toml)
│   └── sub_module.mc     # Other .mc files
```

### 6.2 Common Workflows

```bash
# Create a new project
mcc proj create my-project

# Quick syntax/diagnostic check
mcc check ./my-project

# Parse and show structure
mcc parse ./my-project -f json-pretty

# Build and visualize
mcc build --viz
# Opens circuit.html in browser

# Show what's defined in a file
mcc show file -F src/main.mc

# Find a component definition
mcc show component --name RES

# Search for components matching a pattern
mcc search "CAP" --kind component

# Show instances (what's actually used)
mcc show instances -F src/main.mc -T main

# Export netlist
mcc export netlist src/main.mc --top main --json
```

### 6.3 Diagnosing Errors

```bash
# Get all diagnostics for a file
mcc check path/to/file.mc

# With strict checking
mcc check path/to/file.mc --strict

# Explain a specific error code
mcc explain 1100

# Full diagnostics via RPC (with server running)
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"diagnostics","params":{"uri":"file:///absolute/path/to/file.mc"},"id":1}'

# Get semantic tokens + symbols for a file
curl -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"sem","params":{"uri":"file:///absolute/path/to/file.mc"},"id":1}'
```

### 6.4 Common Error Codes

| Code | Meaning | Typical Cause |
|---|---|---|
| 1001-1004 | Duplicate definition | Same name used twice in scope |
| 1100 | Component not found | Missing `use` import or typo |
| 1101 | Module not found | Module name misspelled |
| 1102 | Interface not found | Interface not imported |
| 1103 | Enum not found | Enum value doesn't exist |
| 1104 | Instance not found | Referencing undefined instance |
| 1503 | Duplicate module | Module defined in multiple files |
| 301-303 | Connection error | Pin count mismatch, wrong IO direction |
| 1200-1202 | Port errors | Undefined port, duplicate port name |
| 2001-2008 | Detector warnings | D1-D8 circuit structure issues |

### 6.5 Validating Library Changes

When modifying mcode library files:

```bash
# 1. Check modified file for syntax errors
mcc check ./path/to/changed.mc

# 2. Parse with library loaded
mcc parse ./path/to/changed.mc --lib mcode

# 3. Build a test project that uses the changed component
cd /Users/dan/work/mo/mcd/projects/hbl
mcc build

# 4. Full rebuild with visualization
mcc build --viz

# 5. Run mcc's internal test suite
cd /Users/dan/work/mo/mcc
cargo test
```

### 6.6 RPC-Based Debug Session

```bash
# Terminal 1: Start server
cd /Users/dan/work/mo/mcc
MCC_SYSTEM_ROOT=./mc cargo run -- start --port 8080 --log-level debug --lib mcode

# Terminal 2: Interact via curl
# Initialize and load project
curl -s -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"set_project_root","params":{"path":"/path/to/project"},"id":1}'

curl -s -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"load_project","params":{"uri":"file:///path/to/project/src/main.mc"},"id":2}'

# Get project symbols
curl -s -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"project_symbols","params":{},"id":3}' | python3 -m json.tool

# Build
curl -s -X POST http://127.0.0.1:8080/rpc \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"build.full","params":{"uri":"file:///path/to/project/src/main.mc","top":"main"},"id":4}'
```

---

## 7. LSP Extension (mcext)

### Architecture

```
VS Code  ←LSP→  mcodels (Rust)  ←HTTP JSON-RPC→  mcc server
(extension)     (tower-lsp)                       (axum :8080)
```

### Debug Configurations

In `/Users/dan/work/mo/mcext/.vscode/launch.json`:

| Config | Purpose |
|---|---|
| **Debug LSP Server** | Launch `mcodels` with `RUST_LOG=trace` |
| **Debug VS Code Extension** | Open new VS Code window with extension loaded |
| **Attach to LSP Server** | Attach debugger to running mcodels process |
| **Debug Extension + LSP Server** | Compound: launch both simultaneously |

```bash
# Build extension
cd /Users/dan/work/mo/mcext
cargo build

# Run LSP server standalone (stdin/stdout)
RUST_LOG=trace cargo run --bin mcodels

# Start extension development host
# Use "Debug Extension + LSP Server" launch config, or:
code --extensionDevelopmentPath=/Users/dan/work/mo/mcext /Users/dan/work/mo/mcd/projects/hbl
```

### Key LSP Features

| Feature | RPC Method Used | Source Module |
|---|---|---|
| Semantic tokens | `sem` | `features/semtok.rs` |
| Go-to-definition | `def` | `features/gotodef.rs` |
| Find references | `refs` | `features/refs.rs` |
| Completions | `project_symbols` + `show.*` | `features/comp.rs` |
| Hover | `show.dump` | `features/hover.rs` |
| Diagnostics | `diagnostics` | `features/diag.rs` |
| Formatting | (internal) | `features/fmt.rs` |
| Inlay hints | (internal) | `features/inhint.rs` |

### Health Checks

```bash
# Check mcc server status
curl -X POST http://127.0.0.1:8080/health

# Check if mcodels is running
ps aux | grep mcodels

# View mcc server log
tail -f ~/.mcode/logs/mcc-server.log

# Check extension output in VS Code
# View → Output → "MCode" channel
```

---

## 8. Configuration Reference

### Global Config (`~/.mcode/config/mcc.yaml`)

```yaml
trace:
  enabled: false
  ast: false
  lexer: false
  parser: false
  visit: false
  pass1: false
  pass2: false
  server: false

parser:
  sort_pins: "pinid"  # pinid | interface

output:
  format: "text"       # text | json | yaml

libs:
  preload:
    - mcode
```

### Project Config (`project.toml`)

```toml
[project]
name = "my-project"
version = "0.1.0"
entry = "src/main.mc"
top_module = "main"

[dependencies]
mcode = "*"

[config.trace]
enabled = false
pass1 = false
pass2 = false
```

### Server Config (`~/.mcode/config/server.yaml`)

```yaml
server:
  host: "127.0.0.1"
  port: 8080
  tls: false
  auth: "none"     # none | basic | token
  max_connections: 100
  request_timeout_ms: 30000

logging:
  level: "info"    # debug | info | warn | error
  file: ""         # empty = stderr only
```
