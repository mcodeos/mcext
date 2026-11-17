# 3. 查找引用（Find References）

## 3.1 功能目标

光标停在符号上，按 Shift+F12 列出所有**引用**该符号的位置。

## 3.2 LSP 协议

- **请求**：`textDocument/references` → `Vec<Location>`
- **请求字段**：`ReferenceParams { include_declaration: bool }`

## 3.3 实现要点

### 3.3.1 符号类型 → 引用集合映射

```rust
fn collect_references(
    state: &WorkspaceState,
    uri: &Url,
    offset: usize,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let symbols = state.symbols(uri)?;
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    
    let refs = match interval.val {
        SymbolType::DeclareClass(sid) | SymbolType::ClassDefinition(sid) => {
            // 全局类引用：跨文件
            let gtable = symbols.global_table.lock().ok()?;
            let ids = gtable.class_id_to_reference_ids.get(&sid).cloned().unwrap_or_default();
            ids.iter()
                .filter_map(|id| {
                    let (uri, span) = gtable.declare_class_id_to_span.get(id)?;
                    Some((uri.clone(), span.clone()))
                })
                .collect()
        }
        SymbolType::DeclareInstance(sid) | SymbolType::InstanceReference(sid) => {
            // 本文件实例引用
            let ids = symbols.local_table.declare_inst_to_inst_ids
                .get(&sid).cloned().unwrap_or_default();
            ids.iter()
                .filter_map(|id| symbols.local_table.inst_id_to_span.get(id).cloned())
                .map(|span| (uri.clone(), span))
                .collect()
        }
        _ => return None,
    };
    
    let rope = state.rope(uri)?;
    let result: Vec<Location> = refs.into_iter()
        .filter_map(|(u, span)| {
            // include_declaration 过滤
            if !include_declaration && interval.val.is_declaration() {
                // 跳过声明
            }
            let start = offset_to_position(span.start, &rope)?;
            let end = offset_to_position(span.end, &rope)?;
            let url = Url::from_file_path(Path::new(&u.to_string())).ok()?;
            Some(Location::new(url, Range::new(start, end)))
        })
        .collect();
    
    Some(result)
}
```

### 3.3.2 跨文件引用

依赖项目索引。`ProjectIndex::lookup_references(symbol_key)` 返回所有引用位置。

### 3.3.3 include_declaration

LSP 允许客户端请求"包括声明"或"纯引用"。mcc 的 `class_id_to_reference_ids` 是否含声明需要确认 —— 当前实现是"仅引用"，需要在 include_declaration=true 时主动加入 `class_id_to_span[sid]`。

## 3.4 测试

### Rust 单测
1. 光标在 `DeclareInstance` → 列出本文件引用 + 声明（按参数）
2. 光标在 `DeclareClass` → 列出跨文件引用
3. `include_declaration=false` → 排除声明位置
4. 未声明符号 → 空列表

### E2E
1. 打开 `crossfile/main.mc`，光标在 `helper_instance` 上
2. Shift+F12
3. 断言：弹出"引用"面板，含 main 和 helper 中的所有位置

## 3.5 当前状态

- ✅ `DeclareInstance` 引用（本文件）
- ✅ `DeclareClass` 引用（本文件）
- ✅ 跨文件引用（走项目索引与 mcc `sem`）
- ❌ `include_declaration` 处理

## 3.6 重构工作

1. 抽 `features::references::resolve`
2. 接入 `ProjectIndex::lookup_references`
3. 正确处理 `include_declaration`
4. 加单测 + E2E

## 3.7 相关 mcc 接口

```rust
pub struct McSemSymbols {
    pub global_table: Mutex<GlobalTable>,
    pub local_table: LocalTable,
    pub symbol_lapper: Lapper,
}

pub struct GlobalTable {
    pub class_id_to_span: HashMap<SymbolId, (McURI, Range<usize>)>,
    pub class_id_to_reference_ids: HashMap<SymbolId, Vec<SymbolId>>,
    pub declare_class_id_to_span: HashMap<SymbolId, (McURI, Range<usize>)>,
    pub interface_id_to_span: HashMap<SymbolId, (McURI, Range<usize>)>,
}

pub struct LocalTable {
    pub declare_inst_to_inst_ids: HashMap<SymbolId, Vec<SymbolId>>,
    pub inst_id_to_span: HashMap<SymbolId, Range<usize>>,
    pub func_id_to_span: HashMap<SymbolId, Range<usize>>,
}
```