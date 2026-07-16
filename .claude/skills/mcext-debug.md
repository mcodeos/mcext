# mcext Debugging

> Debugging techniques, common issues, and diagnostic workflows for the mcext (`mcodels`) LSP extension.

---

## 1. Logging

### Location

`mcext/log.txt` — truncated and rewritten on each `mcodels` startup.

### Log Level

`src/main.rs` defaults to `INFO`. Override via the `RUST_LOG` env var:

```rust
tracing_subscriber::fmt()
    .with_env_filter(
        EnvFilter::from_default_env()           // reads RUST_LOG
            .add_directive(Level::INFO.into()), // defaults to INFO
    )
```

**Enable DEBUG**: set `RUST_LOG=debug` in the shell that launches mcext, or in `.vscode/settings.json`.

### Temporarily Adding INFO Logs

During debugging, promote key `debug!`/`trace!` calls to `info!`, then revert when done:

```rust
// debug-only
info!("did_open: {}", uri.path());
info!("parse_and_publish ENTER: uri={}", uri.path());
info!("sem RPC OK/FAILED for {uri}: ...");
info!("publish_diagnostics: {} diags for {}", n, uri.path());
```

### Key Log Patterns

| Log | Meaning |
|-----|---------|
| `=== MccServer::start called ===` | Init started |
| `Port 8080 is free, will spawn new mcc` | Cold start |
| `Port 8080 is already in use` | Warm start (mcc already running) |
| `mcc server connected at 127.0.0.1:8080` | Write lock released |
| `init_done = true` | Project init done, parsing can begin |
| `parse_and_publish done for X: N diags` | File X published N diagnostics |
| `Retrying N pending diagnostics` | Phase 3 retrying deferred files |
| `parse_and_publish: init not ready` | File deferred to pending (init not done) |
| `sem RPC FAILED` / `load_project failed` / `Network(...)` | mcc crashed or unreachable |

---

## 2. Build & Restart

### Build

```bash
cd /Users/dan/work/mo/mcext
cargo build
```

**Force rebuild** (when cargo doesn't detect changes):
```bash
touch src/server/mod.rs && cargo build
```

### Verify Binary

```bash
strings target/debug/mcodels | grep "expected_log_message"
```

### Restart LSP

```bash
# Kill old process (VS Code will auto-restart mcodels)
pkill -9 -f "target/debug/mcodels"

# Full cold restart (kill mcc child process too)
pkill -9 -f "target/debug/mcc"
pkill -9 -f "target/debug/mcodels"

# Wait for restart
sleep 5

# Verify
ps aux | grep -E "mcodels|target/debug/mcc" | grep -v grep
tail -20 log.txt
```

### Test mcc Connectivity

```bash
curl -s -X POST http://127.0.0.1:8080/rpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"server.info","params":{},"id":1}'
```

---

## 3. Architecture

### Init Phases

```
Phase 1 (write lock, ~3s):  server.start() → clone MccRpcClient → release write lock
Phase 2 (no lock):          init RPC → load_project → project_symbols
Gate:                       init_done.store(true) + init_notify.notify_waiters()
Phase 3 (serial):           drain pending_diagnostics → await each parse_and_publish
```

### Concurrency Guards

| Mechanism | Protects |
|-----------|----------|
| `init_done` (AtomicBool) | Prevents parse_and_publish / F12 from making RPC calls before init completes |
| `init_notify` (Notify) | Wakes all waiting parse_and_publish tasks when init finishes |
| `rpc_lock` (TokioMutex) | Serializes all RPC calls to mcc (single-threaded server) |

### Process Isolation

mcext runs mcc as a child process (`mcc start`), communicating via HTTP RPC on `127.0.0.1:8080`. mcc crashes don't kill the LSP, but the mcc process must be restarted to recover.

---

## 4. Common Issues

### "Files opened after init don't get diagnostics"

**Symptom**: tabs opened after initialization complete have no diagnostics.

**Root cause**: `Notify` is **not sticky** — `notify_waiters()` only wakes currently-waiting tasks. Subsequent `notified()` calls block forever.

**Fix**: Use `AtomicBool` for the sticky check + `Notify` only for wakeup.

### "F12 crashes mcc / load_project fails"

**Symptom**: pressing F12 during init shows `load_project failed: Network(...)` in log.

**Root cause**: `goto_definition`'s on-the-fly `sem` RPC and Phase 2's `load_project` RPC hit mcc concurrently. mcc is single-threaded and can't handle it.

**Fix**: Wrap `goto_definition`'s `sem` call in `rpc_lock`.

### "Only some files get diagnostics, others silently lost"

**Symptom**: 4 files open but only 2 show `parse_and_publish done`.

**Root cause**: `notify_waiters()` wakes all waiters simultaneously → multiple `parse_and_publish` tasks fire concurrent RPCs → mcc crashes.

**Fix**: `rpc_lock` serializes RPC access.

### "F12 can't find library component definitions"

**Symptom**: F12 on `POWER_USB` etc. does nothing.

**Root cause**:
1. `global_declares` is filtered to current-file only (`file_uri == uri_str`), excludes library definitions
2. Project index hardcodes component/interface span as `(0, 0)`

**Fix**:
1. Add `IndexKind::Component` lookup in `declare_class` / `class_ref` handlers
2. Pass and store actual spans in `build_from_mcb_iter`

**Resolution priority**: current file → use imports → standard library

---

## 5. Debugging Workflow

### Verify Basic Health

```bash
# 1. Check processes are running
ps aux | grep -E "mcodels|target/debug/mcc" | grep -v grep

# 2. Check log
cat mcext/log.txt

# 3. Expected output (INFO level):
#    - init_done = true
#    - parse_and_publish done for X: N diags  (one per open file)
```

### Detect Concurrent RPC Conflicts

Any of these patterns appearing together means RPC conflict:
```
parse_and_publish waiting for init    ← shouldn't appear
load_project failed: Network(...)     ← mcc crashed
sem RPC FAILED: Network(...)          ← mcc crashed
```

### Verify F12 / Goto-Def

Check VS Code Output panel → "mcode" for `eprintln!` output (prefixed `F12_DIAG`).

### Verify Project Index

```
worker: UpdateProjectSymbols components=164 interfaces=53 enums=1
```
Counts >0 means the index is populated.
