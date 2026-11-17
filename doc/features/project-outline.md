# 34. 项目大纲视图（Project Outline View）

## 34.1 功能目标

在 VSCode 侧边栏以树形列出整个项目的组件 / 接口 / 枚举，点击节点跳到对应文件位置。

## 34.2 VSCode API

- `vscode.window.registerTreeDataProvider`
- 数据可来自 `workspace/symbol`（LSP），或插件自有的项目索引 RPC

## 34.3 当前状态

❌ 未实现。插件没有注册任何 TreeDataProvider；项目级符号检索目前走 `workspace/symbol`（命令面板里的跨文件符号搜索），不是侧边栏树。

## 34.4 去向

依赖内核的项目索引面给出稳定的项目清单（不只是符号搜索）。在那之前，用 `workspace/symbol` 与文档大纲（[document-symbol.md](./document-symbol.md)，单文件）覆盖同类需求。
