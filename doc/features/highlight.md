# 1. 语义高亮（Semantic Highlighting）

## 1.1 功能目标

让 VSCode 显示**带语义的颜色**：
- `component` 关键字 → 蓝色
- `ComponentName` 类名 → 黄色（驼峰类）
- `component_instance` 实例名 → 浅蓝
- `pin_name` 属性 → 浅黄
- `pin_number` 数字 → 橙色
- `interface` 名 → 紫色
- `enum` 值 → 青色
- 函数名 → 黄
- 注释 → 灰
- 字符串/数字字面量 → 棕

## 1.2 LSP 协议

- **能力声明**：`ServerCapabilities::semantic_tokens_provider`
- **请求方法**：
  - `textDocument/semanticTokens/full`（全量）
  - `textDocument/semanticTokens/delta`（增量，P1）
  - `textDocument/semanticTokens/range`（可视区域）

## 1.3 数据流

```
Backend::semantic_tokens_full(params)
    │
    ▼
state::WorkspaceState::with_tokens(uri)  // 取 McSemTokens
    │
    ▼
features::semantic_tokens::convert(tokens, rope)
    │   // 转换 McSemToken → LSP SemanticToken（处理多行注释）
    │   // 计算 delta 编码
    ▼
SemanticTokens { data: Vec<SemanticToken>, result_id }
```

## 1.4 Legend 定义

```rust
// src/common/legend.rs
pub const LEGEND_TYPE: &[SemanticTokenType] = &[
    SemanticTokenType::STRING,         // 0
    SemanticTokenType::NUMBER,         // 1
    SemanticTokenType::TYPE,           // 2
    SemanticTokenType::CLASS,          // 3
    SemanticTokenType::FUNCTION,       // 4
    SemanticTokenType::INTERFACE,      // 5
    SemanticTokenType::ENUM,           // 6
    SemanticTokenType::TYPE_PARAMETER, // 7
    SemanticTokenType::PARAMETER,      // 8
    SemanticTokenType::VARIABLE,       // 9
    SemanticTokenType::PROPERTY,       // 10
    SemanticTokenType::METHOD,         // 11
    SemanticTokenType::MACRO,          // 12
    SemanticTokenType::KEYWORD,        // 13
    SemanticTokenType::OPERATOR,       // 14
    SemanticTokenType::REGEXP,         // 15
    SemanticTokenType::COMMENT,        // 16
];

pub const LEGEND_MODIFIER: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,    // 0
    SemanticTokenModifier::DEFINITION,     // 1
    SemanticTokenModifier::READONLY,       // 2
    SemanticTokenModifier::STATIC,         // 3
    SemanticTokenModifier::DEPRECATED,     // 4
    SemanticTokenModifier::ABSTRACT,       // 5
    SemanticTokenModifier::ASYNC,          // 6
    SemanticTokenModifier::MODIFICATION,   // 7
    SemanticTokenModifier::DOCUMENTATION,  // 8
    SemanticTokenModifier::DEFAULT_LIBRARY,// 9
];
```

## 1.5 mcode 语义类型映射

| mcode 元素 | LSP TokenType | Index | Modifier |
|---|---|---:|---|
| 关键字 `component`/`interface`/`enum`/`func`/`module`/`io`/`in`/`out`/`pins`/`spec`/`layout`/`left`/`right` | KEYWORD | 13 | |
| 用户定义组件名（声明） | CLASS | 3 | DECLARATION |
| 用户定义组件名（引用） | CLASS | 3 | |
| 用户定义接口名（声明） | INTERFACE | 5 | DECLARATION |
| 用户定义接口名（引用） | INTERFACE | 5 | |
| 用户定义枚举名（声明） | ENUM | 6 | DECLARATION |
| 用户定义枚举值 | ENUM | 6 | |
| 实例名声明 `R442` | VARIABLE | 9 | DECLARATION |
| 实例名引用 `R442` | VARIABLE | 9 | READONLY |
| 引脚号 `u1.1` | NUMBER | 1 | |
| 引脚名 `u1.VDD` | PROPERTY | 10 | |
| 函数名声明 | FUNCTION | 4 | DECLARATION |
| 函数名调用 | FUNCTION | 4 | |
| 函数参数 | PARAMETER | 8 | |
| 字符串字面量 | STRING | 0 | |
| 数字字面量 | NUMBER | 1 | |
| 注释 | COMMENT | 16 | |
| 运算符 `-`/`+`/`->`/`=>`/`:` | OPERATOR | 14 | |

## 1.6 实现要点

### 1.6.1 增量转换（delta encoding）

LSP 规定 `SemanticToken.data` 是相对编码：

```
delta_line:   从上一个 token 所在行的相对行数
delta_start:  同一行内，从上一个 token 起点的相对字符偏移
length:       token 长度
token_type:   legend 索引
token_modifiers_bitset: modifier 位掩码
```

### 1.6.2 多行注释拆分

`token.type_ == 101` 表示多行注释（来自 `work.md` 中 debug 日志），需要拆成多个 token，每个连续行一段。详见 `backend.rs:583-687` 的现有实现。

### 1.6.3 文本/字节坐标转换

mcc 用**字节偏移**，LSP 用**字符偏移**。注意 `rope.try_byte_to_char(offset)` 的转换。

### 1.6.4 result_id 管理

```rust
pub struct TokensState {
    next_id: AtomicU64,
    last: RwLock<HashMap<Url, (u64, Vec<SemanticToken>)>>,
}

impl TokensState {
    pub fn next_id(&self) -> u64 { self.next_id.fetch_add(1, Ordering::Relaxed) }
    pub fn store(&self, uri: Url, id: u64, tokens: Vec<SemanticToken>) {
        self.last.write().insert(uri, (id, tokens));
    }
    pub fn get(&self, uri: &Url) -> Option<(u64, &[SemanticToken])> {
        self.last.read().get(uri).map(|(id, t)| (*id, t.as_slice()))
    }
}
```

## 1.7 测试

### Rust 单测
1. 单行注释 → 1 token
2. 多行注释 → N token（N = 行数）
3. 关键字 `component` → 正确 token_type
4. 引脚访问 `u1.VDD` → 拆成 NUMBER + PROPERTY 两个 token
5. 非法 offset → 返回空（不 panic）

### E2E 测试
打开 fixture，截图对比高亮效果。

## 1.8 配置项

```json
{
  "mcodels.semanticTokens.enabled": {
    "type": "boolean",
    "default": true,
    "description": "Enable semantic highlighting (requires server restart)"
  },
  "mcodels.semanticTokens.fullOnChange": {
    "type": "boolean",
    "default": true,
    "description": "Send full semantic tokens on every change (otherwise delta)"
  }
}
```

## 1.9 主题（package.json）

```json
{
  "contributes": {
    "semanticTokenScopes": [
      {
        "language": "mcode",
        "scopes": {
          "variable.readonly": "variable.modifier.readonly",
          "class": "entity.name.class",
          "interface": "entity.name.interface",
          "keyword": "keyword.control",
          "property": "variable.other.property"
        }
      }
    ],
    "semanticTokenColors": {
      "variable.readonly": "#4FC1FF",
      "class": "#4EC9B0",
      "interface": "#B5CEA8",
      "keyword": "#569CD6",
      "property": "#9CDCFE"
    }
  }
}
```

## 1.10 当前状态

- ✅ `textDocument/semanticTokens/full`
- ✅ `textDocument/semanticTokens/delta`（增量）
- ✅ `textDocument/semanticTokens/range`（只回该范围内的 token）
- ❌ `result_id` 管理（当前传 None）

## 1.11 重构工作

1. 把 `backend.rs::get_semantic_tokens` 抽到 `features/semantic_tokens.rs::convert`
2. 加 `result_id` 管理
3. 加 `semantic_tokens_delta` handler
4. 加单测（参考 §1.7）
5. 加配置项（`mcodels.semanticTokens.enabled`）
6. 加主题贡献（§1.9）