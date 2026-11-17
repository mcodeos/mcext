# 5. 内嵌提示（Inlay Hints）

## 5.1 功能目标

在源码中**直接显示**推断信息，灰色小字，鼠标悬停可看完整内容。

## 5.2 LSP 协议

- **能力**：`inlay_hint_provider: Some(OneOf::Left(true))`
- **请求**：`textDocument/inlayHint` → `Vec<InlayHint>`

## 5.3 InlayHint 字段

```rust
pub struct InlayHint {
    pub position: Position,        // 插入位置
    pub label: InlayHintLabel,     // 显示文本
    pub kind: Option<InlayHintKind>, // Type / Parameter
    pub tooltip: Option<MarkupContent>,
    pub padding_left: Option<bool>,
    pub padding_right: Option<bool>,
    pub text_edits: Option<Vec<TextEdit>>,
    pub data: Option<Value>,
}
```

## 5.4 提示类型

### 5.4.1 类型提示（`InlayHintKind::TYPE`）

```mcode
// 源码
component my {
    A - B
}
// 内嵌提示
component my : chip  // 提示这是一个 Component
```

或：
```mcode
A  // : pin_name[1]
A.1  // pin #1 (VDD)
```

### 5.4.2 参数名提示（`InlayHintKind::PARAMETER`）

```mcode
// 源码
my_func(1, 2, 3)
// 提示
my_func(1 /* a */, 2 /* b */, 3 /* c */)
```

### 5.4.3 引脚号展开

```mcode
// 源码
u1[1:4]
// 提示
u1[1:4]  // pins: 1, 2, 3, 4
```

### 5.4.4 实例类型提示

```mcode
// 源码
name = R442
R442 - GND
// 提示
R442 - GND  // R442: RES(1kΩ)
```

## 5.5 数据流

```
Client: textDocument/inlayHint { uri, range }
    │
    ▼
Backend::inlay_hint(params)
    │
    ▼
features::inlay_hint::compute(state, uri, range)
    │   1. 在 range 内遍历 mcc token / AST 节点
    │   2. 对每个"可提示"位置，构造 InlayHint
    ▼
Vec<InlayHint>
```

## 5.6 实现要点

### 5.6.1 客户端注册

VSCode 1.67+ 才内置支持 inlay hints；老版本需要客户端二次注册。

如果目标版本 ≥ 1.85（按当前 `client/package.json` 已是 1.85+），只需 server 端声明能力。

### 5.6.2 提示缓存

每次输入都重算所有 inlay hints 太贵。VSCode 支持 `resolveProvider`：

```rust
InlayHintOptions {
    resolve_provider: Some(true),
    work_done_progress_options: Default::default(),
}
```

但实际不常用，先不上 resolveProvider。

### 5.6.3 实现示例

```rust
pub fn compute(state: &WorkspaceState, uri: &Url, range: Range) -> Vec<InlayHint> {
    let rope = state.rope(uri).unwrap();
    let offset_start = position_to_offset(range.start, &rope).unwrap_or(0);
    let offset_end = position_to_offset(range.end, &rope).unwrap_or(usize::MAX);
    
    let symbols = state.symbols(uri).unwrap();
    let mut hints = Vec::new();
    
    // 例：实例引用后追加类型
    for (sid, span) in symbols.local_table.inst_id_to_span.iter() {
        if span.start < offset_start || span.end > offset_end {
            continue;
        }
        let inst_type = symbols.local_table.inst_type.get(sid);  // 假设 mcc 暴露
        if let Some(ty) = inst_type {
            let pos = offset_to_position(span.end, &rope).unwrap();
            hints.push(InlayHint {
                position: pos,
                label: InlayHintLabel::LabelParts(vec![
                    InlayHintLabelPart { value: ": ".into(), ..default() },
                    InlayHintLabelPart {
                        value: ty.clone(),
                        tooltip: Some(format!("Instance type: {}", ty).into()),
                        ..default()
                    },
                ]),
                kind: Some(InlayHintKind::TYPE),
                padding_left: Some(true),
                ..default()
            });
        }
    }
    
    hints
}
```

## 5.7 数据源

| 提示 | 数据 |
|---|---|
| 实例类型 | `local_table.inst_id_to_type`（mcc 需暴露） |
| 引脚号 | `McPins::names_to_id` |
| 函数签名 | `local_table.func_id_to_signature` |
| 切片展开 | `local_table.slice_id_to_pins` |

## 5.8 测试

### Rust 单测
1. 实例引用 → 类型提示
2. 引脚访问 → 引脚号提示
3. 切片 → 展开提示
4. range 边界 → 只提示范围内的
5. 字符串内的伪实例 → 不提示

### E2E
1. 打开文件 → 提示出现
2. `Mcode: Toggle Inlay Hints` → 提示消失
3. 鼠标悬停提示 → 显示完整 tooltip

## 5.9 配置项

```json
{
  "mcodels.inlayHints.enabled": {
    "type": "boolean",
    "default": true,
    "description": "Enable inlay hints"
  },
  "mcodels.inlayHints.showInstanceType": {
    "type": "boolean",
    "default": true
  },
  "mcodels.inlayHints.showPinNumber": {
    "type": "boolean",
    "default": true
  },
  "mcodels.inlayHints.showSliceExpansion": {
    "type": "boolean",
    "default": false,
    "description": "Expand name[a:b] to full pin list"
  },
  "mcodels.inlayHints.suppressWhenTypeMatchesName": {
    "type": "boolean",
    "default": true,
    "description": "Don't show hint when name == type (e.g. 'R442: RES' if name starts with R)"
  }
}
```

## 5.10 当前状态

- ❌ 未实现——**声明已撤**：此前 provider 恒回空数组，是空转的承诺面；`mcodels.inlayHintsEnabled` 配置项一并撤除

## 5.11 重构工作

1. 实现 `features::inlay_hint::compute`
2. 在 `ServerCapabilities` 声明 `inlay_hint_provider`
3. 注册 `mcode.toggleInlayHints` 命令
4. 加单测 + E2E
5. 加配置项
6. 视 mcc 接口可用性补齐 `inst_id_to_type` 等查询

## 5.12 相关 mcc 接口

```rust
// mcc 当前是否暴露？需确认
impl McSemSymbols {
    pub fn inst_id_to_type(&self, id: SymbolId) -> Option<String>;
    pub func func_id_to_signature(&self, id: SymbolId) -> Option<String>;
}
```

如果 mcc 没有这些方法，需要补 mcc 接口或换思路（直接从 McInstance 推）。