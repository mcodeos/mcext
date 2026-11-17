# 2. 跳转到定义（Go to Definition）

## 2.1 功能目标

光标停在任意符号上，F12 跳到它的定义处：实例引用跳实例声明、组件名跳组件声明、`use` 路径跳目标文件、跨文件引用跳到别的文件。

## 2.2 LSP 协议

- **能力**：`definition_provider: Some(OneOf::Left(true))`
- **请求**：`textDocument/definition` → `Option<GotoDefinitionResponse>`

## 2.3 当前实现

源码：`mcext/src/features/gotodef.rs`

- `resolve()`：先用 mcc `sem` RPC 取回光标处的符号信息（lapper + `cross_file_targets`），再在插件侧做名称模糊匹配与定义种类过滤（`belongs_to`、def_kinds），命中后定位到定义行并跳过同 span 的自引用。
- `resolve_use_statement()`：`use` 语句上的跳转直接指向目标文件。
- 跨文件命中时读取目标文件并计算目标 `Range`（`cross-file response` 段）。

## 2.4 当前状态

✅ 已实现。本文件跳转、跨文件跳转、`use` 路径跳转均可用。

## 2.5 边界

- 依赖内核 `sem` RPC 提供 `cross_file_targets`；内核不可用时只能落回本文件内的定义定位。
- 定义种类过滤（def_kinds）目前按 mcc 的 kind 名表走，插件侧不再另立一份语义表。

## 2.6 相关 mcc 接口

`sem` RPC（lapper + `cross_file_targets`），补全与本能力共享同一份 lapper 数据。
