# 23. 代码修复建议（Code Actions）

## 23.1 功能目标

在诊断错误上显示一个"灯泡"，点击弹出可执行的修复选项。

## 23.2 LSP 协议

- **能力**：`code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
    code_action_kinds: Some(vec![
        CodeActionKind::QUICKFIX,
        CodeActionKind::REFACTOR,
        CodeActionKind::REFACTOR_EXTRACT,
        CodeActionKind::REFACTOR_INLINE,
        CodeActionKind::REFACTOR_REWRITE,
    ]),
    resolve_provider: Some(true),
  }))`
- **请求**：`textDocument/codeAction` → `Vec<CodeActionOrCommand>`

## 23.3 CodeAction 结构

```rust
pub struct CodeAction {
    pub title: String,
    pub kind: Option<CodeActionKind>,
    pub diagnostics: Option<Vec<Diagnostic>>,
    pub is_preferred: Option<bool>,
    pub disabled: Option<Disabled>,
    pub edit: Option<WorkspaceEdit>,
    pub command: Option<Command>,
    pub data: Option<Value>,
}
```

## 23.4 内置 QuickFix 类型

### 23.4.1 "未声明实例引用" → 提供自动创建

```
Diagnostic: "Instance 'R442' not declared"
QuickFix: "Create instance declaration"
   Edit: 在文件顶部插入 `R442 = RES()::RES()`
```

### 23.4.2 "引脚不存在" → 提供建议的引脚名

```
Diagnostic: "Pin 'VDDC' not found on 'u1' (did you mean 'VDD'?)"
QuickFix: "Change to 'VDD'"
   Edit: replace 'VDDC' with 'VDD'
```

### 23.4.3 "缺分号 / 缺 `}`" → 自动补全

```
Diagnostic: "Expected '}'"
QuickFix: "Insert '}'"
   Edit: insert '}' at end of block
```

### 23.4.4 "未使用的实例" → 提供删除

```
Warning: "Instance 'R442' is never used"
QuickFix: "Remove declaration"
   Edit: delete declaration line
```

## 23.5 实现要点

### 23.5.1 数据流

```
Client: textDocument/codeAction { uri, range, diagnostics }
    │
    ▼
Backend::code_action(params)
    │
    ▼
features::code_action::compute(state, uri, range, diagnostics)
    │   对每个 diagnostic，根据其 code 匹配 fix handlers
    ▼
Vec<CodeActionOrCommand>
```

### 23.5.2 Fix Handler 注册表

```rust
pub trait QuickFixHandler: Send + Sync {
    fn code(&self) -> u32;  // diagnostic.code
    fn title(&self) -> &'static str;
    fn fix(&self, state: &WorkspaceState, uri: &Url, diag: &Diagnostic) -> Option<WorkspaceEdit>;
}

pub struct QuickFixRegistry {
    handlers: Vec<Box<dyn QuickFixHandler>>,
}

impl QuickFixRegistry {
    pub fn new() -> Self {
        Self {
            handlers: vec![
                Box::new(CreateInstanceFix),
                Box::new(RenamePinFix),
                Box::new(InsertMissingBraceFix),
                Box::new(RemoveUnusedFix),
            ],
        }
    }
    
    pub fn fix_for(&self, state: &WorkspaceState, uri: &Url, diag: &Diagnostic) -> Vec<CodeAction> {
        self.handlers.iter()
            .filter(|h| Some(h.code()) == diag.code)
            .filter_map(|h| h.fix(state, uri, diag).map(|edit| CodeAction {
                title: h.title().to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                edit: Some(edit),
                is_preferred: Some(true),
                ..default()
            }))
            .collect()
    }
}
```

### 23.5.3 具体 Fix 示例

```rust
struct RenamePinFix;
impl QuickFixHandler for RenamePinFix {
    fn code(&self) -> u32 { 1305 }  // pin name not found
    
    fn title(&self) -> &'static str { "Change to closest pin name" }
    
    fn fix(&self, state: &WorkspaceState, uri: &Url, diag: &Diagnostic) -> Option<WorkspaceEdit> {
        // 1. 从 diag.message 提取拼错的 pin 名
        let bad_name = extract_pin_name(&diag.message)?;
        
        // 2. 找上下文：哪个 instance 的哪个引脚被访问
        let (instance, _) = find_pin_access_at(uri, diag.range.start, state)?;
        
        // 3. 找该 instance 的合法引脚列表
        let valid_pins = state.instance_pins(uri, instance)?;
        
        // 4. 编辑距离找最近
        let suggestion = find_closest(&bad_name, &valid_pins)?;
        
        // 5. 构造 TextEdit
        let edit = TextEdit::new(diag.range, suggestion.to_string());
        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);
        Some(WorkspaceEdit::new(changes))
    }
}
```

## 23.6 测试

### Rust 单测
1. diagnostic 1305 → RenamePinFix 触发，返回 edit
2. diagnostic 1310 → CreateInstanceFix 触发
3. 无匹配 fix → 空列表
4. 用户接受 fix 后，文件正确修改

### E2E
1. 写错 pin 名 → 红灯泡
2. 点击 → 弹出 fix
3. 选中 → 自动修复

## 23.7 当前状态

- ✅ 已实现：服务端 `code_action` handler；具体 quick fix 从请求自带的诊断派生（如 "Explain E…" 走 `mcode.explain` 命令）

## 23.8 重构工作

1. 定义 fix handler trait
2. 注册 4-6 个常用 fix
3. 实现 `features::code_action::compute`
4. 加单测 + E2E

## 23.9 相关 mcc 接口

需要 mcc 的 diagnostic 携带**结构化的 code** 和**建议**，否则 fix 实现会比较 hacky。

```rust
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub location: Location,
    pub message: String,
    pub code: Option<u32>,
    pub suggestions: Vec<Suggestion>,  // 需要补
}
```