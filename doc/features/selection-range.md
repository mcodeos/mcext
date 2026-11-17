# 24. 选择范围扩展（Selection Range）

## 24.1 功能目标

光标处按扩展键，范围逐级放大：先选中标识符，再扩到它所在的表达式、语句、容器，直到整个 component。

## 24.2 LSP 协议

- **能力**：`selection_range_provider: Some(OneOf::Left(true))`
- **请求**：`textDocument/selectionRange` → `Vec<SelectionRange>`

## 24.3 当前实现

源码：`mcext/src/features/selrange.rs`

`resolve()` 构造一个**跨度链**：最内层是光标周围的标识符词（byte span），然后是所有包含光标的 lapper span，按跨度从小到大排列——每一级严格包含前一级。最后一次性把 byte span 换算成 `Position`。

## 24.4 当前状态

✅ 已实现。链的构造在插件侧完成，数据来源仍是内核 lapper（与补全、跳转共享同一份符号表）。

## 24.5 边界

- lapper 尚未覆盖的语法结构不产生级别；链的深度取决于内核给出的 span 粒度。
