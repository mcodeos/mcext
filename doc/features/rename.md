# 8. 重命名符号（Rename Symbol）

## 8.1 功能目标

F2 改名符号，所有引用同步更新。

## 8.2 LSP 协议

- **能力**：`rename_provider: Some(OneOf::Left(true))`
- **请求**：
  - `textDocument/rename` → `WorkspaceEdit`
  - `textDocument/prepareRename` → `Option<Range>`（P2，先验证可重命名）

## 8.3 实现要点

### 8.3.1 可重命名的范围

| 符号 | 可重命名 | 备注 |
|---|---|---|
| 用户组件声明 | ✅ | 跨文件 |
| 用户接口声明 | ✅ | 跨文件 |
| 用户枚举 | ✅ | 跨文件 |
| 函数名 | ✅ | 跨文件 |
| 实例名 | ✅ | 仅本文件 |
| 引脚名 | ⚠️ | 需确认 mcc 是否允许 |
| 内置件（`RES`/`CAP`） | ❌ | 拒绝，返回 `null` |
| 关键字 | ❌ | 拒绝 |
| 系统库组件 | ❌ | 不在用户控制范围 |

### 8.3.2 prepareRename

```rust
async fn prepare_rename(&self, params: TextDocumentPositionParams) -> Result<Option<Range>> {
    let state = self.state.lock().await;
    let symbols = state.symbols(&params.text_document.uri)?;
    let rope = state.rope(&params.text_document.uri)?;
    let offset = position_to_offset(params.position, &rope)?;
    
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    
    if !is_renamable(interval.val) {
        return Ok(None);  // 客户端禁用 rename
    }
    
    let range = interval_range(interval, &rope)?;
    Ok(Some(range))
}
```

### 8.3.3 rename

```rust
async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
    let uri = params.text_document_position.text_document.uri;
    let new_name = params.new_name;
    
    if !is_valid_identifier(&new_name) {
        return Ok(None);  // 拒绝非法名
    }
    
    let state = self.state.lock().await;
    let symbols = state.symbols(&uri)?;
    let rope = state.rope(&uri)?;
    let offset = position_to_offset(params.text_document_position.position, &rope)?;
    
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    if !is_renamable(interval.val) {
        return Ok(None);
    }
    
    let edits = match interval.val {
        SymbolType::DeclareClass(sid) | SymbolType::ClassDefinition(sid) => {
            // 跨文件：全局表
            collect_global_renames(&state, sid, &new_name)?
        }
        SymbolType::InstanceReference(sid) | SymbolType::DeclareInstance(sid) => {
            // 本文件
            collect_local_renames(&state, &uri, sid, &new_name)?
        }
        _ => return Ok(None),
    };
    
    // 按文件分组
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for (target_uri, span) in edits {
        let start = offset_to_position(span.start, &rope_for(&target_uri, &state)?)?;
        let end = offset_to_position(span.end, &rope_for(&target_uri, &state)?)?;
        changes.entry(target_uri).or_default().push(
            TextEdit::new(Range::new(start, end), new_name.clone())
        );
    }
    
    Ok(Some(WorkspaceEdit::new(changes)))
}
```

### 8.3.4 跨文件重命名

```rust
fn collect_global_renames(
    state: &WorkspaceState,
    sid: SymbolId,
    new_name: &str,
) -> Option<Vec<(Url, Range<usize>)>> {
    let symbols_global = state.global_symbols().lock().ok()?;
    
    // 1. 声明本身
    let decl = symbols_global.declare_class_id_to_span.get(&sid)?.clone();
    let mut all = vec![decl];
    
    // 2. 所有引用
    if let Some(refs) = symbols_global.class_id_to_reference_ids.get(&sid) {
        for ref_id in refs {
            if let Some((uri, span)) = symbols_global.declare_class_id_to_span.get(ref_id) {
                all.push((uri.clone(), span.clone()));
            }
        }
    }
    
    Some(all)
}
```

### 8.3.5 名称校验

```rust
fn is_valid_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !RESERVED_KEYWORDS.contains(&name)
}
```

## 8.4 测试

### Rust 单测
1. 重命名本文件实例 → 全部引用替换
2. 重命名跨文件组件 → 多文件 WorkspaceEdit
3. 重命名内置件 `RES` → 返回 None
4. 重命名为空字符串 → 返回 None
5. 重命名为非法字符 → 返回 None

### E2E
1. 光标在 `R442`，按 F2，输入 `R100`
2. 所有 `R442` → `R100`
3. 跨文件验证：`crossfile/main.mc` 中也替换

## 8.5 当前状态

- ✅ 已实现：`textDocument/rename`（改名并同步引用）＋ `prepareRename`（校验可改范围）

## 8.6 重构工作

1. 实现 `features::rename::prepare_rename`
2. 实现 `features::rename::rename`
3. 接入项目索引的跨文件支持
4. 名称校验
5. 加单测 + E2E

## 8.7 相关 mcc 接口

```rust
pub struct LocalTable {
    pub inst_id_to_span: HashMap<SymbolId, Range<usize>>,
}

pub struct GlobalTable {
    pub class_id_to_span: HashMap<SymbolId, (McURI, Range<usize>)>,
    pub class_id_to_reference_ids: HashMap<SymbolId, Vec<SymbolId>>,
    pub declare_class_id_to_span: HashMap<SymbolId, (McURI, Range<usize>)>,
}
```

## 25. 准备重命名（Prepare Rename）

**LSP**：`textDocument/prepareRename` → `Option<Range>`

**行为**：重命名前先问内核"这个名字能不能改、范围到哪"；不能改的位置直接不弹输入框。

**当前实现**：服务端 `prepare_rename` handler 调 `features::highlight::prepare`（光标周围的标识符范围），返回 `PrepareRenameResponse::Range`。

**当前状态**：✅ 已实现。
