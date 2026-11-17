# 26. 链接定义（Document Links）

## 26.1 功能目标

在源码里把可点击的引用变成链接（例如注释里的 `see also: USB.mc`，或 `use` 指向的文件），点击直接跳过去。

## 26.2 LSP 协议

- `textDocument/documentLink` → `Vec<DocumentLink>`

## 26.3 当前状态

❌ 未实现——**声明已撤**。此前的 handler 对每个请求恒回 `None`，是空转的承诺面；capability 侧也从未声明过 `document_link_provider`。

## 26.4 导航由谁承担

- `use` 路径跳转：`textDocument/definition`（见 [goto-definition.md](./goto-definition.md)），光标停在 `use` 行即跳目标文件。
- 符号跳转：hover 展示定义位置，F12 跳转。

即导航能力已有，只是不以 DocumentLink 的形式提供。

## 26.5 去向

待内核侧给出可枚举的引用面（哪些位置是可点击引用）后重新立面并实装；在此之前插件不声明该能力。
