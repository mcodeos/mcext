# 10. 文档高亮（Document Highlight）

## 10.1 目标

光标停在符号上时，自动高亮该符号在当前文件中的所有出现（类似 VSCode 选中一个变量时其他同名变量也高亮）。

## 10.2 LSP 协议

- **能力**：`document_highlight_provider: Some(OneOf::Left(true))`
- **请求**：`textDocument/documentHighlight` → `Vec<DocumentHighlight>`

## 10.3 DocumentHighlight 结构

```rust
pub struct DocumentHighlight {
    pub range: Range,
    pub kind: Option<DocumentHighlightKind>,  // Text / Read / Write
}
```

## 10.4 实现要点

复用 `references` 的逻辑，但限定到当前文件：

```rust
pub fn compute(state: &WorkspaceState, uri: &Url, offset: usize) -> Vec<DocumentHighlight> {
    let symbols = state.symbols(uri).unwrap();
    let rope = state.rope(uri).unwrap();
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    
    let refs = match interval.val {
        SymbolType::DeclareInstance(sid) | SymbolType::InstanceReference(sid) => {
            symbols.local_table.declare_inst_to_inst_ids
                .get(&sid).cloned().unwrap_or_default()
        }
        SymbolType::DeclareClass(sid) | SymbolType::ClassDefinition(sid) => {
            // 本文件的 class 引用（不含声明）
            symbols.global_table.lock().ok()?
                .class_id_to_reference_ids.get(&sid).cloned().unwrap_or_default()
                .into_iter()
                .filter_map(|id| {
                    let (u, _) = symbols.global_table.lock().ok()?.declare_class_id_to_span.get(&id)?;
                    if u == uri { Some(id) } else { None }
                })
                .collect()
        }
        _ => vec![],
    };
    
    refs.iter()
        .filter_map(|id| {
            let span = symbols.local_table.inst_id_to_span.get(id)?;
            let start = offset_to_position(span.start, &rope)?;
            let end = offset_to_position(span.end, &rope)?;
            Some(DocumentHighlight {
                range: Range::new(start, end),
                kind: Some(DocumentHighlightKind::TEXT),
            })
        })
        .collect()
}
```

## 10.5 当前状态

- ✅ 已实现：光标处符号的所有出现（本地计算，无 RPC）

## 10.6 重构工作

1. 实现 `features::document_highlight::compute`
2. 复用 references 的部分代码
3. 加单测 + E2E