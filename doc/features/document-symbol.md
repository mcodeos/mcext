# 9. 文档大纲（Document Outline）

## 9.1 功能目标

VSCode 左侧"大纲"视图显示当前文件结构。

## 9.2 LSP 协议

- **能力**：`document_symbol_provider: Some(OneOf::Left(true))`
- **请求**：`textDocument/documentSymbol` → `Vec<DocumentSymbol>` 或 `Vec<SymbolInformation>`

推荐用 `DocumentSymbol`（支持层级树）。

## 9.3 DocumentSymbol 结构

```rust
pub struct DocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub tags: Option<SymbolTag>,
    pub deprecated: Option<bool>,
    pub range: Range,
    pub selection_range: Range,
    pub children: Option<Vec<DocumentSymbol>>,
}
```

## 9.4 mcode 符号层级

```
▼ Component: my_chip              [SymbolKind::CLASS]
    ▶ Pins                         [SymbolKind::PROPERTY]
      - VCC                        [SymbolKind::VARIABLE]
      - GND                        [SymbolKind::VARIABLE]
    ▶ Spec                         [SymbolKind::NAMESPACE]
      - Vgs                        [SymbolKind::VARIABLE]
      - Rdson                      [SymbolKind::VARIABLE]
    ▶ Attributes                   [SymbolKind::NAMESPACE]
      - Vth                        [SymbolKind::VARIABLE]
▶ Interface: my_iface             [SymbolKind::INTERFACE]
▶ Enum: my_enum                   [SymbolKind::ENUM]
    - VAL_A                        [SymbolKind::ENUM_MEMBER]
    - VAL_B                        [SymbolKind::ENUM_MEMBER]
▶ Function: helper                [SymbolKind::FUNCTION]
```

## 9.5 实现要点

### 9.5.1 数据源

mcc 已暴露 `mcb_iter_*` 接口，可列出当前项目的所有符号。但 LSP `documentSymbol` 是**单文件**范围，所以：

```rust
pub fn compute(uri: &Url, state: &WorkspaceState) -> Option<Vec<DocumentSymbol>> {
    let symbols = state.symbols(uri)?;
    let rope = state.rope(uri)?;
    
    let mut top_level = Vec::new();
    
    // 当前文件中的所有 declare_class / declare_instance / func 等
    let local = &symbols.local_table;
    
    // 组件声明
    for (sid, span) in local.declare_inst_to_inst_ids.iter() {
        // ...
    }
    
    // 接口
    // ...
    
    // 枚举
    // ...
    
    // 函数
    // ...
    
    Some(top_level)
}
```

### 9.5.2 children 嵌套

`pins` / `spec` / `layout` 等块作为组件的 children；每个引脚作为 pins 的 children。

需要 mcc 暴露嵌套信息，或在 LSP 侧根据文本位置推断（同一 component 内的 declare 都归到这个 component 的 children 下）。

### 9.5.3 selection_range

`range` 是整个 symbol 的范围，`selection_range` 是光标应该跳转的位置（通常是名字本身的位置）。

```rust
DocumentSymbol {
    name: "my_chip".into(),
    range: Range::new(component_start, component_end),
    selection_range: Range::new(component_name_start, component_name_end),
    children: ...,
    kind: SymbolKind::CLASS,
    detail: None,
    tags: None,
    deprecated: None,
}
```

## 9.6 测试

### Rust 单测
1. 单 component 文件 → 顶层 1 个 CLASS 节点
2. component + interface + enum + func → 顶层 4 个节点
3. 引脚作为 children
4. 空文件 → 空列表

### E2E
1. 打开 fixture 文件
2. 断言大纲面板显示正确的层级

## 9.7 当前状态

- ✅ 已实现：单文件结构树（组件/接口/枚举/函数层级）

## 9.8 重构工作

1. 实现 `features::document_symbol::compute`
2. 处理嵌套层级
3. 加单测 + E2E

## 9.9 相关 mcc 接口

需要确认 mcc 暴露：
```rust
pub fn mcb_iter_components() -> impl Iterator<Item = McComponent>;
pub fn mcb_iter_interfaces() -> impl Iterator<Item = McInterface>;
pub fn mcb_iter_enums() -> impl Iterator<Item = McEnumDef>;
```

可能需要按 URI 过滤（仅当前文件的）。