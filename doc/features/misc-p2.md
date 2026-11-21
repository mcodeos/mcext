# P2 杂项功能汇总

> P2 功能不紧迫，按需实现。本文档集中描述实现思路，便于后续查阅。

---

## 24. 选择范围扩展（Selection Range）

**LSP**：`textDocument/selectionRange` → `Vec<SelectionRange>`

**目标**：双击选择 `u1`，再按 Ctrl+Shift+Right 扩展到 `u1.VDD`，再到 `u1.VDD - GND`，再扩到整个 component。

**实现**：根据 mcc token / AST 的嵌套关系构造嵌套的 `SelectionRange` 树。

**当前状态**：❌

---

## 25. 准备重命名（Prepare Rename）

**LSP**：`textDocument/prepareRename` → `Option<Range>`

**目标**：在不可重命名的符号上按 F2，不弹输入框；可重命名时高亮整个 token。

**实现**：详见 [rename.md §8.3.2](./rename.md#832-preparerename)。

**当前状态**：❌

---

## 26. 链接定义（Document Links）

**LSP**：`textDocument/documentLink` → `Vec<DocumentLink>`

**目标**：在注释中识别 `see also: USB.mc` 这种引用，点击跳转到对应文件。

**实现**：扫描文档中的 `file://` URI 或相对路径，构造 DocumentLink。

**当前状态**：❌

---

## 27. Pull 模式诊断（Pull Diagnostics）

**LSP**：`workspace/diagnostic`

**目标**：客户端主动拉取诊断，而非 server 主动推送。减少大量错误时的推送风暴。

**实现**：详见 [diagnostics.md §3.4.4](./diagnostics.md#344-pull-诊断p2)。

**当前状态**：❌

---

## 30. 进度通知（Progress Notifications）

**LSP**：`$/progress`

**目标**：构建、解析等长时间操作时显示进度条。

**实现**：

```rust
let token = client.progress_begin("Building...").await?;
client.progress_report(token, "Parsing files", 50).await?;
client.progress_end(token, "Done").await?;
```

**当前状态**：❌

---

## 34. 项目大纲视图（Project Outline View）

**目标**：VSCode 侧边栏 TreeView 显示项目所有组件 / 接口 / 枚举。

**实现**：
- `client/src/views/project-outline.ts` 用 `vscode.window.registerTreeDataProvider`
- 数据来自 `workspace/symbol` 或自定义 LSP 请求
- 节点点击跳转到对应文件

**当前状态**：❌

---

## 37. 内嵌预览（Preview Netlist）

**目标**：右侧 webview 实时显示构建后的网表。

**实现**：
- `client/src/views/preview.ts`
- 监听 build 命令
- 渲染 SVG / Mermaid 图

**当前状态**：❌

---

## 38. 字符串/字符选择（Word Selection）

**LSP**：内置，不需要 server。

**目标**：双击 `R442` 选中整个标识符；双击 `"hello"` 选中整个字符串。

**实现**：在 `language-configuration.json` 的 `wordPattern` 中定义：

```json
{
  "wordPattern": "[A-Za-z_][A-Za-z0-9_]*"
}
```

字符串字面量由 VSCode 内置 word selection rules 自动处理（基于 quote 字符）。

**当前状态**：❌