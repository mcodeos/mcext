# 3. 语法诊断（Diagnostics）

## 3.1 功能目标

mcc 解析时产生的错误 / 警告，实时显示在 VSCode：

- 编辑器右侧红/黄竖条
- 文件底部状态栏显示错误数
- "问题"面板（Ctrl+Shift+M）列出所有错误
- F8 跳到下一个错误

## 3.2 LSP 协议

### 推送模式（当前使用）
- `textDocument/publishDiagnostics` — server 主动调用 `client.publish_diagnostics`

### 拉取模式（P2 增强）
- `workspace/diagnostic` — client 主动拉取

## 3.3 数据流

```
Client: textDocument/didChange (params)
    │
    ▼
Backend::did_change(params)
    │
    ▼
state::apply_incremental(uri, changes)         // 更新 Rope
state::schedule_reparse(uri, version)         // debounce 150ms
    │
    ▼ (after debounce)
state::reparse(uri, version)
    │  spawn_blocking:
    │     mcc::mcc_load_from_string(&uri, &text)
    │     mcc::mcc_diagnose(&uri) → Vec<Diagnostic>
    │     mcc::mcc_query(&uri) → Option<ParserResult>
    ▼
Backend::publish_diagnostics(uri, diagnostics, version)
    │
    ▼
Client: 在编辑器中显示红/黄竖条
```

## 3.4 实现要点

### 3.4.1 Debounce

```rust
pub struct ReparseScheduler {
    pending: Mutex<HashMap<Url, oneshot::Sender<()>>>,
    debounce: Duration,
}

impl ReparseScheduler {
    pub fn schedule<F>(&self, uri: Url, version: i32, on_fire: F)
        where F: FnOnce() + Send + 'static
    {
        let mut pending = self.pending.lock().unwrap();
        if let Some(tx) = pending.remove(&uri) {
            let _ = tx.send(());  // 取消上一次的 fire
        }
        let (tx, rx) = oneshot::channel();
        pending.insert(uri.clone(), tx);
        
        let debounce = self.debounce;
        tokio::spawn(async move {
            tokio::time::sleep(debounce).await;
            if rx.await.is_ok() {
                // 未被取消 → fire
                on_fire();
            }
        });
    }
}
```

### 3.4.2 诊断收集

```rust
// features::diagnostics::collect
pub fn collect(state: &WorkspaceState, uri: &Url) -> Vec<Diagnostic> {
    let rope = state.rope(uri).unwrap();
    let uri_str = uri.path();
    let mc_uri = McURI::from(uri_str);
    
    mcc::mcc_diagnose(&mc_uri)
        .iter()
        .filter_map(|item| {
            let start = offset_to_position(item.location.pos as usize, &rope)?;
            let end = offset_to_position((item.location.pos + item.location.len) as usize, &rope)?;
            let severity = match item.level {
                DiagnosticLevel::Error => Some(DiagnosticSeverity::ERROR),
                DiagnosticLevel::Warning => Some(DiagnosticSeverity::WARNING),
                DiagnosticLevel::Info => Some(DiagnosticSeverity::INFORMATION),
                DiagnosticLevel::Hint => Some(DiagnosticSeverity::HINT),
            };
            Some(Diagnostic::new(
                Range::new(start, end),
                severity,
                Some(NumberOrString::String(item.code.to_string())),
                Some("mcc".into()),
                item.message.clone(),
                None,
                None,
            ))
        })
        .collect()
}
```

### 3.4.3 解析后的发布顺序

为了避免 LSP 收到乱序消息（高 version 的诊断比低 version 早到），需要 version 校验：

```rust
async fn publish_diagnostics(&self, uri: Url, version: Option<i32>, diagnostics: Vec<Diagnostic>) {
    // 读 state 中的最新 version
    let current_version = self.state.document_version(&uri).await;
    
    if let (Some(sent), Some(curr)) = (version, current_version) {
        if sent < curr {
            debug!("stale diagnostics for {}, skip", uri);
            return;
        }
    }
    
    self.client.publish_diagnostics(uri, diagnostics, version).await;
}
```

### 3.4.4 Pull 诊断（P2）

```rust
// 在 ServerCapabilities 中声明：
diagnostic_provider: Some(DiagnosticOptions {
    identifier: Some("mcc".into()),
    inter_file_dependencies: true,
    workspace_diagnostics: true,
}),
```

handler：
```rust
async fn diagnostic(&self, params: WorkspaceDiagnosticParams) -> Result<WorkspaceDiagnosticReport> {
    // 遍历所有打开文档的 diagnostics
    let items: Vec<WorkspaceDocumentDiagnosticReport> = self.state
        .documents()
        .filter_map(|uri| {
            let diags = features::diagnostics::collect(&self.state, &uri)?;
            Some(WorkspaceDocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: None,
                    items: diags,
                },
            }))
        })
        .collect();
    
    Ok(WorkspaceDiagnosticReport::WorkspaceReports(WorkspaceDiagnosticReportPartialResult {
        items,
    }))
}
```

## 3.5 测试

### Rust 单测
1. 正确语法 → 空 diagnostics
2. 缺分号 → 1 diagnostic，ERROR
3. 未声明实例引用 → 1 diagnostic，WARNING（来自 mcc 的引用检查）
4. 同一 URI 多次发布 → 第二次覆盖第一次
5. Stale version → 被忽略

### E2E
1. 输入有语法错误的 mcode
2. 等待 200ms（debounce）
3. 断言 "问题" 面板有红色条目
4. F8 → 光标跳到错误位置
5. 修复错误 → 红色消失

## 3.6 配置项

```json
{
  "mcodels.diagnostics.debounceMs": {
    "type": "number",
    "default": 150,
    "minimum": 0,
    "maximum": 5000,
    "description": "Debounce duration for re-parsing and diagnostics (ms)"
  },
  "mcodels.diagnostics.openOutputOnError": {
    "type": "boolean",
    "default": true,
    "description": "Open output panel when diagnostics occur"
  },
  "mcodels.diagnostics.pullMode": {
    "type": "boolean",
    "default": false,
    "description": "Use pull diagnostics (workspace/diagnostic) instead of push"
  }
}
```

## 3.7 当前状态

- ✅ Push 模式（已有）
- ✅ `mcc_diagnose` 已用
- ✅ Debounce（默认 150 ms，`mcodels.diagnosticsDebounceMs` 可调）
- ❌ Version 校验（无，乱序可能发生）
- ❌ Pull 模式（未实现）
- ❌ 状态栏错误计数（未做）

## 3.8 重构工作

1. 抽 `features::diagnostics::collect`
2. 加 `ReparseScheduler`（debounce）
3. 加 version 校验
4. 加 Pull 模式 handler
5. 加状态栏错误计数（参见 [status-bar.md](./status-bar.md)）
6. 加配置项
7. 加单测 + E2E

## 3.9 已知 mcc 接口

```rust
pub fn mcc_diagnose(uri: &McURI) -> Vec<Diagnostic>;
pub fn mcc_diagnose_all() -> Vec<Diagnostic>;
```

`Diagnostic` 结构（来自 mcc）：
```rust
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub location: Location,
    pub message: String,
    pub code: Option<u32>,
}
```

## 27. Pull 模式诊断（Pull Diagnostics）

**LSP**：`workspace/diagnostic`

**行为**：客户端主动拉取诊断，而不是服务端推送；错误很多时可减少推送风暴。

**当前状态**：❌ 未实现——服务端只走 push 通道（`textDocument/publishDiagnostics`），没有 `workspace/diagnostic` handler。

**替代**：push 通道已做 debounce（默认 150 ms），风暴问题在客户端侧已被压住；pull 模式待内核诊断面支持按需拉取后实装。

---

## 30. 进度通知（Progress Notifications）

**LSP**：`$/progress`

**行为**：构建、解析等长操作期间在编辑器里显示进度条。

**当前状态**：❌ 未实现。LSP 能力的 `work_done_progress_options` 已声明，但插件没有发起任何 `$/progress` 序列。

**替代**：构建过程写 `MCode Build` 输出通道（`client/src/extension.ts` 的 `buildOutput`），错误另走专属 DiagnosticCollection。
