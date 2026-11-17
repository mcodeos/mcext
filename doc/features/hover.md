# 7. 悬停文档（Hover）

## 7.1 功能目标

鼠标悬停在符号上 → 显示信息气泡。

## 7.2 LSP 协议

- **能力**：`hover_provider: Some(HoverProviderCapability::Simple(true))`
- **请求**：`textDocument/hover` → `Option<Hover>`

## 7.3 Hover 结构

```rust
pub struct Hover {
    pub contents: HoverContents,
    pub range: Option<Range>,
}

pub enum HoverContents {
    Scalar(MarkupContent),
    Array(Vec<MarkupContent>),
    Markup(MarkupContent),
}
```

## 7.4 实现要点

### 7.4.1 符号类型 → Hover 内容映射

| 符号 | 内容 |
|---|---|
| 用户组件声明 | 组件注释 + spec 概要 + 引脚列表 |
| 用户接口声明 | 接口成员列表 |
| 用户枚举 | 枚举值列表 |
| 函数声明 | 函数签名 + 注释 |
| 实例引用 | 实例类型 + 实例声明位置 |
| 引脚访问 | 引脚号 + 引脚名 + IO 类型 |
| 内置件（`RES`/`CAP`） | 简要说明 + 默认值 |
| 关键字 | 语法定义链接 |

### 7.4.2 实现示例

```rust
pub fn compute_hover(state: &WorkspaceState, uri: &Url, offset: usize) -> Option<Hover> {
    let symbols = state.symbols(uri)?;
    let rope = state.rope(uri)?;
    let interval = symbols.symbol_lapper.find(offset, offset+1).next()?;
    
    let content = match interval.val {
        SymbolType::DeclareClass(sid) => {
            let comp = state.lookup_component(sid)?;
            markdown_hover_component(&comp, state)
        }
        SymbolType::InstanceReference(sid) => {
            let inst = symbols.local_table.inst_metadata.get(&sid)?;
            format!("**{}** — {}\n\n{}",
                inst.name, inst.component_class, inst.doc_comment)
        }
        // ...
        _ => return None,
    };
    
    let range = symbol_interval_to_range(interval, &rope)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: Some(range),
    })
}

fn markdown_hover_component(comp: &McComponent, state: &WorkspaceState) -> String {
    let mut s = format!("## component {}\n\n", comp.name);
    if let Some(doc) = &comp.doc_comment {
        s.push_str(doc);
        s.push_str("\n\n");
    }
    s.push_str("### Pins\n\n");
    for pin in &comp.pins {
        s.push_str(&format!("- **{}** (#{}) `{:?}`\n",
            pin.name, pin.id, pin.io_type));
    }
    s
}
```

## 7.5 数据源

```rust
pub struct McComponent {
    pub name: String,
    pub doc_comment: Option<String>,
    pub pins: McPins,
    pub attributes: Vec<McAttribute>,
    pub spec: Option<McSpec>,
}
```

## 7.6 测试

### Rust 单测
1. 悬停组件名 → 显示 pin 列表
2. 悬停实例 → 显示类型
3. 悬停引脚 → 显示引脚详情
4. 悬停空白 → None
5. Markdown 格式正确

### E2E
1. 打开文件，鼠标悬停 `R442`
2. 弹出气泡：含 "RES(1kΩ)"
3. 鼠标悬停 `USB::USB`
4. 弹出气泡：含组件规格

## 7.7 当前状态

- ✅ 已实现：本地解析优先（定义行 ＋ 位置）；符号属四种具名类之一且 mcc 可达时追加 `show.*` 卡片

## 7.8 重构工作

1. 实现 `features::hover::compute`
2. 接入 McComponent 的 doc_comment / spec
3. 加 Markdown 渲染辅助
4. 加单测 + E2E

## 7.9 相关 mcc 接口

```rust
pub struct McComponent {
    pub name: String,
    pub doc_comment: Option<String>,  // 需确认存在
    pub pins: McPins,
    pub spec: Option<McSpec>,
    // ...
}
```

如果没有 `doc_comment` 字段，需在 mcc 侧补（语法层 comment 关联到 declaration）。