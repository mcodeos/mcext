# 14. 代码折叠（Folding Range）

## 14.1 目标

VSCode 自动给 `component { ... }`、`interface { ... }`、`pins = [ ... ]` 等块生成折叠范围。

## 14.2 LSP 协议

- **能力**：`folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true))`
- **请求**：`textDocument/foldingRange` → `Vec<FoldingRange>`

## 14.3 实现方式（两条路）

### 14.3.1 VSCode Language Configuration（推荐，无需 server）

在 `language-configuration.json` 中定义 folding markers：

```json
{
  "folding": {
    "markers": {
      "start": "^\\s*(component|interface|enum|func|module)\\s+\\w+",
      "end": "^\\s*\\}"
    }
  }
}
```

VSCode 自动识别。

### 14.3.2 LSP server 方式（更精确）

```rust
pub fn compute(rope: &Rope) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let mut brace_stack: Vec<usize> = vec![];
    
    for (offset, ch) in rope.chars().enumerate() {
        match ch {
            '{' => brace_stack.push(offset),
            '}' => {
                if let Some(start) = brace_stack.pop() {
                    let start_line = rope.try_byte_to_line(start).ok()?;
                    let end_line = rope.try_byte_to_line(offset).ok()?;
                    if end_line > start_line {
                        ranges.push(FoldingRange {
                            start_line: start_line as u32,
                            start_character: None,
                            end_line: end_line as u32,
                            end_character: None,
                            kind: Some(FoldingRangeKind::Region),
                            collapsed_text: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    
    ranges
}
```

## 14.4 优先级

| 方式 | 优先级 | 适用 |
|---|---|---|
| Language Configuration | 首选 | 简单正则匹配足够 |
| LSP foldingRange | 备选 | 需要 AST 级精确判断 |

推荐用 **方式 1**（已在 [language-config.md](./language-config.md) 中定义）。

## 14.5 当前状态

- ✅ 已实现：括号与注释折叠（本地计算，无 RPC）

## 14.6 重构工作

在 `language-configuration.json` 中添加 folding 段（参见 [language-config.md](./language-config.md) §20.2）。