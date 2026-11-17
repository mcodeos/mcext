# 11. 自动格式化（Format Document）

## 11.1 功能目标

美化 mcode 源码，统一风格。

## 11.2 LSP 协议

- **能力**：`document_formatting_provider: Some(OneOf::Left(true))`
- **请求 1**：`textDocument/formatting` → `Vec<TextEdit>`（全文）
- **请求 2**：`textDocument/rangeFormatting` → `Vec<TextEdit>`（选区）

## 11.3 格式规则（建议默认）

```yaml
indent: 4 spaces
max_line_length: 100
insert_final_newline: true
trim_trailing_whitespace: true
```

### 规则集

| 规则 | 规则内容 |
|---|---|
| R1 | `component X {` 后换行 + 缩进 |
| R2 | `}` 单独占一行 |
| R3 | 运算符 `-`/`+`/`->`/`=>` 两侧空格 |
| R4 | `,` 后空格，前无空格 |
| R5 | `pins = [...]` 内每个 pin 一行 |
| R6 | `pins = [ ... ]` 多 pin 时每个 `[a:b]` 块独立行 |
| R7 | `pin` 块内 `=` 对齐（可选） |
| R8 | 长行在运算符后换行 |
| R9 | 注释 `//` 与代码间 1 空格 |
| R10 | 嵌套 component 缩进 |

### 示例

```mcode
// 输入
component my_chip{pins=[io 1 = VCC,io 2 = GND,io 3 = IN,io 4 = OUT]}
A-B
C->D

// 输出
component my_chip {
    pins = [
        io 1 = VCC,
        io 2 = GND,
        io 3 = IN,
        io 4 = OUT
    ]
}
A - B
C -> D
```

## 11.4 实现策略

**核心问题**：mcc 是否有自己的格式化器？

- 如果**有**：调 mcc 接口，拿格式化结果，比较差异，构造 TextEdit。
- 如果**没有**：在 LSP 侧实现一个简单的格式化器。

按现状 mcc 没有暴露格式化器（`mcc_*` 接口集未含），所以**走第二条路**：在 `features::formatting` 中实现。

### 实现思路

```
输入 mcode 文本
    │
    ▼
Lexer（用 mcc 的 token 流，或自己写一个简单 lexer）
    │
    ▼
Parser → AST（用 mcc 的 parser，或简化版）
    │
    ▼
Pretty Printer → 格式化文本
    │
    ▼
diff(orig, formatted) → Vec<TextEdit>
```

**注意**：这里复用 mcc 的 lexer / parser 是最稳妥的（避免重新发明语法）。但 mcc 的 AST 不一定适合 pretty-print，可能需要重新组织。

**简化方案**：不解析整个 AST，只识别**块结构**（`{ }` / `[ ]` / `()`），按规则重排 + 重缩进：

```rust
pub fn format(text: &str, opts: &FormatOptions) -> String {
    let mut out = String::with_capacity(text.len());
    let mut indent = 0;
    let mut prev_token = Token::None;
    
    for token in lex(text) {
        match token.kind {
            Token::OpenBrace => {
                out.push_str(&format!("{}{{\n", indent_str(indent)));
                indent += 1;
            }
            Token::CloseBrace => {
                indent -= 1;
                out.push('\n');
                out.push_str(&format!("{}}}\n", indent_str(indent)));
            }
            Token::Newline => { /* 规范化 */ }
            Token::Op(op) => {
                out.push(' ');
                out.push_str(op);
                out.push(' ');
            }
            // ...
        }
    }
    
    out
}
```

### 块结构识别

要支持 R5 (`pins = [...]` 每行一个)，需要识别 `[ ]` 内的逗号分隔项。简单方法：维护一个**括号深度**栈，每进 `[`/`(`/`{` 时新行，每出时新行。

### TextEdit 构造

```rust
pub fn diff_to_edits(orig: &str, formatted: &str) -> Vec<TextEdit> {
    // 用 similar crate 或 myers diff 算法
    // 比较两文本，返回 LSP TextEdit 列表
}
```

简化做法：如果原文与格式化结果差异巨大，全文替换：
```rust
vec![TextEdit::new(
    Range::new(Position::new(0, 0), Position::new(u32::MAX, u32::MAX)),
    formatted,
)]
```

VSCode 支持这种"全文替换"的 TextEdit。

## 11.5 数据流

```
Client: textDocument/formatting { uri, options: { tabSize, insertSpaces } }
    │
    ▼
Backend::formatting(params)
    │
    ▼
state::document_text(uri) → String
    │
    ▼
features::formatting::format(&text, opts)
    │
    ▼
features::formatting::diff_edits(&orig, &formatted, &rope)
    │
    ▼
Vec<TextEdit>
```

## 11.6 测试

### Rust 单测
1. 单行 component → 多行格式化
2. 不规则缩进 → 统一 4 空格
3. 缺空格 `A-B` → `A - B`
4. 块边界正确（`}` 单独行）
5. `pins = [...]` 多 pin 拆行
6. 注释位置保留
7. 配置 `tabSize=2` → 用 2 空格

### E2E
1. 输入混乱 mcode
2. Shift+Alt+F
3. 断言：格式符合预期

## 11.7 配置项

```json
{
  "mcodels.format.tabSize": {
    "type": "number",
    "default": 4,
    "minimum": 1,
    "maximum": 16
  },
  "mcodels.format.insertSpaces": {
    "type": "boolean",
    "default": true
  },
  "mcodels.format.maxLineLength": {
    "type": "number",
    "default": 100
  },
  "mcodels.format.insertFinalNewline": {
    "type": "boolean",
    "default": true
  },
  "mcodels.format.trimTrailingWhitespace": {
    "type": "boolean",
    "default": true
  },
  "mcodels.format.alignAssignments": {
    "type": "boolean",
    "default": false,
    "description": "Align '=' in pin/attr blocks (visual alignment)"
  }
}
```

VSCode 也支持：
```json
"[mcode]": {
  "editor.formatOnSave": true,
  "editor.tabSize": 4
}
```

## 11.8 当前状态

- ✅ 已实现：全文与选区格式化（`textDocument/formatting` ＋ `rangeFormatting`）

## 11.9 重构工作

1. 实现 `features::formatting::format`
2. 实现 `features::formatting::diff_edits`
3. 注册 `document_formatting_provider` capability
4. 加单测（覆盖 5+ 规则）
5. 加 E2E
6. 加配置项
7. 加 `Mcode: Format Document` 命令（`mcode.formatDocument`）

## 11.10 与 mcc 的关系

mcc 若已有内部格式化器（哪怕未公开），应优先使用；当前 mcc 没有暴露格式化接口 → LSP 侧自实现。

如果未来 mcc 提供 `mcc_format(text: &str, opts: FormatOptions) -> String`，LSP 应切换到调用 mcc，避免双实现。