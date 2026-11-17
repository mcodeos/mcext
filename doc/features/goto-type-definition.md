# 16. 类型定义（Go to Type Definition）

## 16.1 功能目标

光标停在实例或值上，跳到它的**类型**定义处（组件定义、接口定义、枚举定义），而不是它自己的声明处。

与 [goto-definition.md](./goto-definition.md) 的区别：definition 跳到"这个东西是什么"，typeDefinition 跳到"这个东西的类型是什么"。

## 16.2 LSP 协议

- **能力**：`type_definition_provider`
- **请求**：`textDocument/typeDefinition` → `Option<GotoDefinitionResponse>`

## 16.3 当前状态

❌ 未实现。服务端既没有 `type_definition_provider` 声明，也没有对应 handler。

## 16.4 为何未做

要跳类型，内核必须能给出"该位置的类型"，并把类型解析到定义位置。当前内核的 `sem` 面提供的是定义与引用，没有独立的类型面；插件若自行推断类型，就是自造语义（U739 的边界：真值归内核）。

## 16.5 去向

待内核提供类型面后重新立面并实装。中间形态可退而求其次——类型信息随 hover 展示（见 [hover.md](./hover.md)），跳转仍走 definition。
