# 15/16. 跳转到实现 / 类型定义

## 15.1 跳转到实现（Go to Implementation）

**目标**：光标停在接口名上，跳转到该接口的实现者。

**LSP**：`textDocument/implementation` → `Vec<Location>`

**mcode 场景**：
- `interface Power { VCC, GND }` → 跳到所有 `component X : Power { ... }` 的位置
- mcc 是否支持接口实现关系需要确认

**实现**：遍历所有组件，找出实现指定接口的。

```rust
pub fn compute(state: &WorkspaceState, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let symbols = state.symbols(uri)?;
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    
    let iface_id = match interval.val {
        SymbolType::DeclareInterface(sid) => sid,
        _ => return None,
    };
    
    let iface_name = symbols.global_table.lock().ok()?
        .interface_id_to_name.get(&iface_id)?;
    
    state.implementations_of(iface_name)  // ProjectIndex 查询
        .into_iter()
        .filter_map(|(u, span)| {
            let rope = state.rope(&u)?;
            let s = offset_to_position(span.start, &rope)?;
            let e = offset_to_position(span.end, &rope)?;
            Some(Location::new(u, Range::new(s, e)))
        })
        .collect()
}
```

## 15.2 跳转到类型定义（Go to Type Definition）

**目标**：光标停在实例引用上，跳到该实例类型的组件声明。

**LSP**：`textDocument/typeDefinition` → `Vec<Location>`

**mcode 场景**：
- `R442` → `component RES { ... }` （如果是自定义 RES）

**实现**：

```rust
pub fn compute(state: &WorkspaceState, uri: &Url, offset: usize) -> Option<Vec<Location>> {
    let symbols = state.symbols(uri)?;
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    
    let type_name = match interval.val {
        SymbolType::InstanceReference(sid) | SymbolType::DeclareInstance(sid) => {
            symbols.local_table.inst_type.get(&sid)?
        }
        SymbolType::DeclareClass(sid) | SymbolType::ClassDefinition(sid) => {
            symbols.global_table.lock().ok()?
                .class_id_to_name.get(&sid)?
        }
        _ => return None,
    };
    
    // 查 ProjectIndex 找类型声明
    let target = state.project_index.lookup_type(type_name)?;
    Some(vec![Location::new(target.0, target.1)])
}
```

## 15.3 当前状态

- ❌ 未实现

## 15.4 重构工作

1. 实现 `features::goto_implementation::compute`
2. 实现 `features::goto_type_definition::compute`
3. 加 mcc 接口（如 `interface_id_to_name`）
4. 加单测 + E2E

## 15.5 注意

如果 mcc 还没有接口实现关系的语义，这两项暂时只能是 P3。