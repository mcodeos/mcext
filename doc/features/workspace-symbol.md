# 17. 工作区符号搜索（Workspace Symbol）

## 17.1 功能目标

Ctrl+T 弹出输入框，跨文件模糊搜索符号。

## 17.2 LSP 协议

- **能力**：`workspace_symbol_provider: Some(OneOf::Left(true))`
- **请求**：`workspace/symbol` → `Vec<SymbolInformation>`

参数：
```rust
pub struct WorkspaceSymbolParams {
    pub query: String,  // 用户输入的查询字符串
}
```

## 17.3 SymbolInformation 结构

```rust
pub struct SymbolInformation {
    pub name: String,
    pub kind: SymbolKind,
    pub tags: Option<SymbolTag>,
    pub deprecated: Option<bool>,
    pub location: Location,
    pub container_name: Option<String>,
}
```

## 17.4 实现要点

### 17.4.1 数据源

`ProjectIndex::symbol_to_loc` 缓存全局符号 → 位置映射。

### 17.4.2 模糊匹配

```rust
pub fn fuzzy_match(query: &str, name: &str) -> bool {
    // 子序列匹配
    let mut qchars = query.chars();
    let mut qcurr = qchars.next();
    for c in name.chars() {
        if Some(c) == qcurr {
            qcurr = qchars.next();
        }
    }
    qcurr.is_none()
}
```

更精细的：用 `fuzzy-matcher` crate 或 `nucleo`（VSCode 内部用的）。

### 17.4.3 实现示例

```rust
pub fn search(state: &WorkspaceState, query: &str) -> Vec<SymbolInformation> {
    let index = state.project_index.read();
    
    let mut results: Vec<SymbolInformation> = index.symbol_to_loc.iter()
        .filter(|(k, _)| fuzzy_match(query, &k.name))
        .map(|(k, (uri, range))| SymbolInformation {
            name: k.name.clone(),
            kind: symbol_kind_from(&k.kind),
            tags: None,
            deprecated: None,
            location: Location::new(uri.clone(), *range),
            container_name: k.container.clone(),
        })
        .collect();
    
    // 按相关性排序
    results.sort_by(|a, b| {
        let score_a = relevance_score(&query, &a.name);
        let score_b = relevance_score(&query, &b.name);
        score_b.partial_cmp(&score_a).unwrap_or(Ordering::Equal)
    });
    
    results.truncate(100);  // 限制最大返回数
    results
}

fn relevance_score(query: &str, name: &str) -> f32 {
    // 1. 完整前缀匹配 > 子串匹配 > 子序列匹配
    if name.starts_with(query) { return 1.0; }
    if name.contains(query) { return 0.8; }
    0.5
}
```

## 17.5 测试

### Rust 单测
1. 查询 `USB` → 列出所有 `USB` 相关组件
2. 查询 `cap` → 大小写不敏感，列出 `Cap`、`CAP_*` 等
3. 空查询 → 列出全部
4. 不存在的符号 → 空列表
5. 跨文件：fixture 中放 2 个同名符号 → 都返回

### E2E
1. Ctrl+T
2. 输入 `USB`
3. 断言：弹窗显示匹配项 + 选中跳转

## 17.6 当前状态

- ✅ 已实现：`workspace/symbol` 走项目索引（`ProjectIndex`）

## 17.7 重构工作

1. 维护 `ProjectIndex::symbol_to_loc`
2. 实现 `features::workspace_symbol::search`
3. 加单测 + E2E

## 17.8 相关 mcc 接口

```rust
pub fn mcb_iter_components() -> impl Iterator<Item = McComponent>;
pub fn mcb_iter_interfaces() -> impl Iterator<Item = McInterface>;
pub fn mcb_iter_enums() -> impl Iterator<Item = McEnumDef>;
pub fn mcb_iter_modules() -> impl Iterator<Item = McModule>;
```

遍历这些，构建索引。注意：
- `McComponent.name` 是字符串 ID
- 需要附带 URI + 声明位置的 span
- mcc 是否暴露"声明位置"？若仅暴露 `McComponent` 而不含位置，需补 mcc 接口。