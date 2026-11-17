# 6. 自动补全（Auto-Complete）

## 6.1 功能目标

光标停在源码中时，弹出一个下拉列表，按 Enter 自动补全。

## 6.2 触发场景

| 光标前文本 | 触发后 | 候选 |
|---|---|---|
| `comp` | `component` | 关键字 |
| `component ` | `<类名>` | 系统组件 + 项目组件 |
| `interface ` | `<接口名>` | 系统接口 + 项目接口 |
| `enum ` | `<枚举名>` | 系统枚举 + 项目枚举 |
| `func ` | `<函数名>` | 用户函数 + 系统函数 |
| `u1.` | `<引脚>` | `u1` 的所有引脚 |
| `USB:` | `<成员>` | USB 模块成员 |
| `name[` | `[a:b]` 切片 | 切片语法模板 |
| `name::` | `<类型>` | 已声明的类型 |
| `pin` 块内 | io/in/out | 类型关键字 |
| 任意位置 | - | 关键字（模糊匹配） |

## 6.3 LSP 协议

- **能力**：`completion_provider: Some(CompletionOptions { ... })`
- **请求**：`textDocument/completion`
- **响应**：`CompletionResponse::Array(Vec<CompletionItem>)` 或 `List(CompletionList)`
- **触发字符**：`trigger_characters: Some(vec![".".to_string(), ":".to_string(), "[".to_string()])`

## 6.4 CompletionItem 字段

| 字段 | 类型 | 用途 |
|---|---|---|
| `label` | string | 显示文字 |
| `kind` | `CompletionItemKind` | 图标类别 |
| `detail` | string? | 详细信息（右侧小字） |
| `documentation` | MarkupContent? | 悬停文档 |
| `insert_text` | string? | 插入文本（可与 label 不同） |
| `insert_text_format` | `InsertTextFormat` | PlainText / Snippet |
| `text_edit` | TextEdit? | 替换当前 word |
| `sort_text` | string? | 排序权重 |
| `filter_text` | string? | 模糊匹配用 |
| `additional_text_edits` | Vec<TextEdit>? | 额外的自动补全（如自动加 import） |

## 6.5 数据流

```
Client: textDocument/completion { uri, position, context: { triggerKind, triggerCharacter } }
    │
    ▼
Backend::completion(params)
    │
    ▼
state::resolve_context(uri, position) → CompletionContext
    │   // 解析光标前的 token 序列，识别：
    │   //   - 关键字补全
    │   //   - 实例.引脚
    │   //   - 模块::成员
    │   //   - 类型::构造
    ▼
features::completion::resolve(context, state) → Vec<CompletionItem>
    │   按 context.kind 分发到不同的 resolve_xxx
    ▼
CompletionResponse
```

## 6.6 实现要点

### 6.6.1 CompletionContext 识别

```rust
pub enum CompletionContext {
    Keyword { prefix: String, scope: KeywordScope },
    ComponentName { prefix: String },
    InterfaceName { prefix: String },
    EnumName { prefix: String },
    FunctionName { prefix: String },
    InstancePinAccess { instance: String, prefix: String },
    ModuleMemberAccess { module: String, prefix: String },
    TypeConstructor { type_name: String },
    PinBlock { prefix: String },
    Slice { prefix: String },
}

pub fn resolve_context(
    rope: &Rope,
    offset: usize,
    trigger_char: Option<&str>,
) -> Option<CompletionContext> {
    // 向后扫描最近的非空白 token
    // ...
}
```

### 6.6.2 关键字补全

```rust
fn resolve_keyword(prefix: &str, scope: KeywordScope) -> Vec<CompletionItem> {
    KEYWORDS.iter()
        .filter(|kw| kw.starts_with(prefix))
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(KEYWORD_DOC.get(kw).cloned().unwrap_or_default()),
            insert_text: Some(kw.to_string()),
            sort_text: Some(format!("0_{}", kw)),  // 关键字优先
            ..Default::default()
        })
        .collect()
}

const KEYWORDS: &[&str] = &[
    "component", "interface", "enum", "func", "module",
    "pins", "spec", "layout", "left", "right",
    "io", "in", "out", "inout",
];
```

### 6.6.3 实例引脚补全（依赖项目索引）

```rust
fn resolve_instance_pins(
    state: &WorkspaceState,
    instance: &str,
    prefix: &str,
    uri: &Url,
) -> Vec<CompletionItem> {
    // 1. 在本文件查找 instance 的声明
    let symbols = state.symbols(uri)?;
    let sid = find_instance_id(&symbols, instance)?;
    
    // 2. 从 McPins 拉引脚列表
    let pins = state.instance_pins(uri, sid)?;
    
    // 3. 转 CompletionItem
    pins.into_iter()
        .filter(|p| p.name.starts_with(prefix))
        .map(|p| CompletionItem {
            label: p.name.clone(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(format!("pin {} ({:?})", p.id, p.io_type)),
            documentation: Some(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!("Pin **{}** (#{})\n\nIO type: {:?}", p.name, p.id, p.io_type),
            }),
            insert_text: Some(p.name),
            sort_text: Some(format!("1_{}", p.name)),
            ..Default::default()
        })
        .collect()
}
```

### 6.6.4 触发字符

```rust
// ServerCapabilities::completion_provider
CompletionOptions {
    trigger_characters: Some(vec![
        ".".to_string(),   // instance.pin
        ":".to_string(),   // module::member
        "[".to_string(),   // slice start
    ]),
    resolve_provider: Some(true),  // 支持二次 resolve（取详细文档）
    completion_item: None,
    work_done_progress_options: Default::default(),
    all_commit_characters: None,
}
```

### 6.6.5 Snippet 形式的补全

对 `component <name> { pins = [ /* */ ] }` 这类样板，用 `InsertTextFormat::Snippet`：

```rust
CompletionItem {
    label: "component".to_string(),
    kind: Some(CompletionItemKind::SNIPPET),
    insert_text_format: Some(InsertTextFormat::Snippet),
    insert_text: Some("component ${1:name} {\n    pins = [\n        ${2:io 1 = P1}\n    ]\n}".to_string()),
    documentation: Some(MarkupContent {
        kind: MarkupKind::Markdown,
        value: "**Component template**\n\nCreate a new component with one IO pin.".into(),
    }),
    ..Default::default()
}
```

### 6.6.6 completionItem/resolve

如果 `resolve_provider: Some(true)`，客户端在用户**选中**某项时再次请求 server，取详细信息（更丰富的 hover 文档）。

```rust
async fn completion_resolve(&self, mut item: CompletionItem) -> Result<CompletionItem> {
    if let Some(detail_kind) = &item.data {
        if let Ok(resolved) = state.lookup_detail(detail_kind).await {
            item.documentation = Some(resolved.documentation);
            item.detail = Some(resolved.detail);
            item.additional_text_edits = Some(resolved.additional_edits);
        }
    }
    Ok(item)
}
```

## 6.7 数据源

| 候选类型 | 数据来源 |
|---|---|
| 关键字 | 硬编码列表 |
| 系统组件 | `mcb_iter_components()` 过滤 system library |
| 项目组件 | `mcb_iter_components()` 过滤 project |
| 系统接口 | `mcb_iter_interfaces()` |
| 项目接口 | 同上 |
| 枚举 | `mcb_iter_enums()` |
| 函数 | 当前文件的 `local_table.func_id_to_signature` |
| 实例引脚 | `McPins::names_to_id` |
| 模块成员 | （mcc 接口待补；可从 project 索引查） |

## 6.8 测试

### Rust 单测
1. 关键字前缀 `com` → 包含 `component`，不含 `interface`
2. 实例引脚：`u1.` → 列出 `u1` 的所有引脚
3. 模块成员：`USB:` → 列出 USB 模块成员
4. 模糊匹配：`fun` → 包含 `func`、`function_name`
5. 排序：关键字 < 本文件符号 < 全局符号

### E2E
1. 输入 `comp` → 弹下拉，选 `component` → 自动补全关键字
2. 输入 `u1.V` → 弹下拉，选 `VDD` → 自动插入
3. 选某项按 Tab → 触发 snippet 占位符跳转
4. 测试 trigger 字符 `.` `:` `[`

## 6.9 配置项

```json
{
  "mcodels.completion.triggerChars": {
    "type": "array",
    "items": { "type": "string" },
    "default": [".", ":", "["],
    "description": "Characters that trigger completion"
  },
  "mcodels.completion.enableSnippets": {
    "type": "boolean",
    "default": true,
    "description": "Enable snippet completions for templates"
  },
  "mcodels.completion.enableSystemLib": {
    "type": "boolean",
    "default": true,
    "description": "Include system library symbols in completion"
  },
  "mcodels.completion.maxItems": {
    "type": "number",
    "default": 100,
    "description": "Maximum number of completion items"
  }
}
```

## 6.10 当前状态

- ✅ 已实现：分层补全——RPC 权威路径（内核分层响应 P1–P5/Member/Net）＋ 本地降级（P1–P4，近似时标 `is_incomplete`）

## 6.11 重构工作

1. 实现 `features::completion::resolve`
2. 实现 `CompletionContext` 识别
3. 实现 `completion_resolve`
4. 补 mcc 接口（如果 `mcb_iter_components` 等的过滤不够灵活）
5. 加配置项
6. 加单测 + E2E

## 6.12 相关 mcc 接口

```rust
pub fn mcb_iter_components() -> impl Iterator<Item = McComponent>;
pub fn mcb_iter_interfaces() -> impl Iterator<Item = McInterface>;
pub fn mcb_iter_enums() -> impl Iterator<Item = McEnumDef>;
pub fn mcb_iter_modules() -> impl Iterator<Item = McModule>;
```