# mcext 功能集合

> mcode 语言插件 = VSCode 扩展 = LSP server（Rust）+ Extension Client（TS）+ 配置 / 主题 / 命令 / 文档
>
> 本文档**穷举**扩展应当提供给用户的能力，并指明：
> - 功能编号 / 中文名 / 英文名
> - LSP 协议 / VSCode API 入口
> - 后端实现路径（指向 mcc 接口）
> - 实现状态：✅ 已实现 / ⚠️ 部分实现 / ❌ 未实现 / ⚙️ VSCode 内置
>
> **本表是本仓唯一的实现真源**：状态以实装（`src/` 里的实现与 capability 声明）为准，
> 对不上的一律改本表，不在别处另立说法。README 与 roadmap 都从本表派生。
> - 优先级：P0 必备 / P1 应该 / P2 加分
> - 验收标准

---

## 0. 功能矩阵总览

| # | 中文名 | 英文名 | LSP 入口 | 实现 | 优先级 | 文档 |
|---|---|---|---|---|---|---|
| 1 | 语义高亮 | Semantic Highlighting | `textDocument/semanticTokens/full+delta+range` | ✅ | P0 | [highlight.md](./features/highlight.md) |
| 2 | 跳转到定义 | Go to Definition | `textDocument/definition` | ✅ | P0 | [goto-definition.md](./features/goto-definition.md) |
| 3 | 查找引用 | Find References | `textDocument/references` | ✅ | P0 | [references.md](./features/references.md) |
| 4 | 语法诊断 | Diagnostics | `textDocument/publishDiagnostics` | ✅ | P0 | [diagnostics.md](./features/diagnostics.md) |
| 5 | 类型/参数内嵌提示 | Inlay Hints | `textDocument/inlayHint` | ❌ | P1 | [inlay-hints.md](./features/inlay-hints.md) |
| 6 | 自动补全 | Auto-Complete | `textDocument/completion` | ✅ | P0 | [completion.md](./features/completion.md) |
| 7 | 悬停文档 | Hover | `textDocument/hover` | ✅ | P1 | [hover.md](./features/hover.md) |
| 8 | 重命名符号 | Rename Symbol | `textDocument/rename` | ✅ | P1 | [rename.md](./features/rename.md) |
| 9 | 文档大纲 | Document Outline | `textDocument/documentSymbol` | ✅ | P1 | [document-symbol.md](./features/document-symbol.md) |
| 10 | 高亮当前符号 | Document Highlight | `textDocument/documentHighlight` | ✅ | P2 | [document-highlight.md](./features/document-highlight.md) |
| 11 | 自动格式化 | Format Document | `textDocument/formatting` + `rangeFormatting` | ✅ | P0 | [formatting.md](./features/formatting.md) |
| 12 | 注释切换 | Toggle Line Comment | `editor.action.commentLine` | ✅ | P0 | [comment-toggle.md](./features/comment-toggle.md) |
| 13 | 块注释切换 | Toggle Block Comment | `editor.action.blockComment` | ❌ | P1 | [comment-toggle.md](./features/comment-toggle.md) |
| 14 | 代码折叠 | Folding | `textDocument/foldingRange` | ✅ | P2 | [folding.md](./features/folding.md) |
| 15 | 跳转到实现 | Go to Implementation | `textDocument/implementation` | ❌ | P2 | [goto-implementation.md](./features/goto-implementation.md) |
| 16 | 类型定义 | Go to Type Definition | `textDocument/typeDefinition` | ❌ | P2 | [goto-type-definition.md](./features/goto-type-definition.md) |
| 17 | 工作区符号搜索 | Workspace Symbol | `workspace/symbol` | ✅ | P1 | [workspace-symbol.md](./features/workspace-symbol.md) |
| 18 | 代码片段 | Snippets | `contributes.snippets` | ✅ | P1 | [snippets.md](./features/snippets.md) |
| 19 | 语法高亮（兜底） | Syntax Highlighting (TextMate) | `contributes.grammars` | ✅ | P0 | [textmate.md](./features/textmate.md) |
| 20 | 括号匹配/自动闭合 | Bracket Matching | VSCode language config | ✅ | P0 | [language-config.md](./features/language-config.md) |
| 21 | 括号着色 | Bracket Pair Colorization | VSCode setting | ⚙️ | P0 | (内置) |
| 22 | 自动缩进 | Auto-Indent | VSCode language config | ✅ | P1 | [language-config.md](./features/language-config.md) |
| 23 | 代码修复建议 | Code Actions | `textDocument/codeAction` | ✅ | P1 | [code-actions.md](./features/code-actions.md) |
| 24 | 选择范围扩展 | Selection Range | `textDocument/selectionRange` | ✅ | P2 | [selection-range.md](./features/selection-range.md) |
| 25 | 准备重命名 | Prepare Rename | `textDocument/prepareRename` | ✅ | P2 | [rename.md](./features/rename.md) |
| 26 | 链接定义 | Document Links | `textDocument/documentLink` | ❌ | P2 | [document-links.md](./features/document-links.md) |
| 27 | Pull 模式诊断 | Pull Diagnostics | `workspace/diagnostic` | ❌ | P2 | [diagnostics.md](./features/diagnostics.md) |
| 28 | 状态栏指示 | Status Bar | VSCode API | ✅ | P1 | [status-bar.md](./features/status-bar.md) |
| 29 | 命令面板命令 | Commands | `contributes.commands` | ✅ | P1 | [commands.md](./features/commands.md) |
| 30 | 进度通知 | Progress Notifications | `$/progress` | ❌ | P1 | (内嵌各 feature) |
| 31 | 配置项 | Settings | `contributes.configuration` | ✅ | P0 | [settings.md](./features/settings.md) |
| 32 | 键绑定 | Keybindings | `contributes.keybindings` | ✅ | P1 | [keybindings.md](./features/keybindings.md) |
| 33 | 主题色 / 语义令牌主题 | Theme | `contributes.semanticTokenScopes`+`Colors` | ✅ | P1 | [theme.md](./features/theme.md) |
| 34 | 项目资源管理器视图 | Project Outline View | VSCode TreeView | ❌ | P2 | [project-outline.md](./features/project-outline.md) |
| 35 | 编译/构建入口 | Build Project | VSCode command | ✅ | P1 | [build-command.md](./features/build-command.md) |
| 36 | 错误码跳转 | Jump to Error | `editor.action.gotoNextError` | ⚙️ | P0 | (内置走 diagnostics) |
| 37 | 内嵌 `.mc` 预览 | Preview Netlist | VSCode webview | ✅ | P2 | [preview.md](./features/preview.md) |
| 38 | 字符串/字符选择 | Word Selection | VSCode language config | ❌ | P2 | [language-config.md](./features/language-config.md) |

> 共 **38 项**功能：✅ 已实现 **27** 项 · ⚙️ VSCode 内置 **2** 项 · ❌ 未实现 **9** 项。
> 未实现的 9 项各自在 §2 或对应 `features/*.md` 里写明是什么、为何未做、有无替代路径。

---

## 1. 按优先级分组

> 名单由 §0 矩阵的「优先级」列派生，不另立一份清单。

**P0 必备**：1 语义高亮 · 2 跳转到定义 · 3 查找引用 · 4 语法诊断 · 6 自动补全 · 11 自动格式化 · 12 注释切换 · 19 语法高亮（兜底） · 20 括号匹配/自动闭合 · 21 括号着色 · 31 配置项 · 36 错误码跳转

**P1 应该**：5 类型/参数内嵌提示 · 7 悬停文档 · 8 重命名符号 · 9 文档大纲 · 13 块注释切换 · 17 工作区符号搜索 · 18 代码片段 · 22 自动缩进 · 23 代码修复建议 · 28 状态栏指示 · 29 命令面板命令 · 30 进度通知 · 32 键绑定 · 33 主题色 / 语义令牌主题 · 35 编译/构建入口

**P2 加分**：10 高亮当前符号 · 14 代码折叠 · 15 跳转到实现 · 16 类型定义 · 24 选择范围扩展 · 25 准备重命名 · 26 链接定义 · 27 Pull 模式诊断 · 34 项目资源管理器视图 · 37 内嵌 `.mc` 预览 · 38 字符串/字符选择

---

## 2. 详细功能规格

> 每个功能的详细规格在 `./features/` 子目录。本节给概要。

### 2.1 语义高亮（Semantic Tokens）

**目标**：区分 `component` 关键字、`Component` 类、`component_instance` 实例、`pin_name`、`pin_number`、`label`、`interface`、`enum` 等。

**LSP**：
- `textDocument/semanticTokens/full`
- `textDocument/semanticTokens/delta`
- `textDocument/semanticTokens/range`

**Legend 类型**（已在 `semtoken.rs`）：
```
STRING, NUMBER, TYPE, CLASS, FUNCTION, INTERFACE, ENUM,
TYPE_PARAMETER, PARAMETER, VARIABLE, PROPERTY, METHOD,
MACRO, KEYWORD, OPERATOR, REGEXP, COMMENT
```

**mcode 语义类型 → LSP Token 映射**：
| mcode 元素 | LSP TokenType | TokenModifier |
|---|---|---|
| `component` / `interface` / `enum` / `func` / `module` 关键字 | KEYWORD | |
| 用户定义组件名 (`Component`) | CLASS | |
| 用户定义接口名 (`Interface`) | INTERFACE | |
| 用户定义枚举名 (`Enum`) | ENUM | |
| 实例引用 (`u1`, `R442`) | VARIABLE | (READONLY) |
| 实例名声明 (`name = u1`) | VARIABLE | DECLARATION |
| 引脚号 (`u1.1`) | NUMBER | |
| 引脚名 (`u1.VDD`) | PROPERTY | |
| 函数名 | FUNCTION | |
| 函数参数 | PARAMETER | |
| 字符串字面量 | STRING | |
| 数字字面量 | NUMBER | |
| 注释 | COMMENT | |
| 运算符 (`-`, `+`, `->`) | OPERATOR | |

**delta 支持**：
- 每次 `full` 返回 `result_id`
- 客户端切换 / 重连时 `delta` 请求只传增量
- `range` 请求只回**该范围内的** token（此前整篇返回）

**详见** [features/highlight.md](./features/highlight.md)

---

### 2.2 跳转到定义（Go to Definition）

**行为**：
- 光标停在 `R442` → 跳到 `R442` 实例声明所在文件 + 行
- 光标停在 `u1.1` → 跳到 `u1` 声明，并高亮 `1` 引脚
- 光标停在 `USB::USB` → 跳到 `USB` 组件声明

**LSP**：
- `textDocument/definition` → `GotoDefinitionResponse`

**验收**：
- ✅ 本文件跳转
- ✅ 跨文件跳转（走 mcc `sem` 的 `cross_file_targets`）
- ✅ 引脚号 `u1.1` 定位到实例声明并高亮引脚

**详见** [features/goto-definition.md](./features/goto-definition.md)

---

### 2.3 查找引用（Find References）

**行为**：光标停在 `u1`，列出所有引用 `u1` 的位置（含本文件 + 其他文件）。

**LSP**：
- `textDocument/references` → `Vec<Location>`

**已有实现**：`DeclareInstance` 引用完整；`DeclareClass` 等种类覆盖不全。

**验收**：
- ✅ `DeclareInstance` 引用
- ⚠️ `DeclareClass` 引用（覆盖不全）
- ✅ 跨文件引用（走项目索引与 mcc `sem`）

---

### 2.4 语法诊断（Diagnostics）

**行为**：mcc 解析错误（语法 / 语义）实时显示在编辑器。

**LSP**：
- 主动：`textDocument/publishDiagnostics`（push）
- 被动：`workspace/diagnostic`（pull，P2）

**现状**：
- 诊断经 `mcc_diagnose` 取回；mcc 侧是全工作区诊断，单文件 change 也会扫全项目（mcc 限制）
- 已做 debounce（默认 150 ms，`mcodels.diagnosticsDebounceMs` 可调），按键不会成风暴
- Pull 模式（`workspace/diagnostic`）未实现

**详见** [features/diagnostics.md](./features/diagnostics.md)

---

### 2.5 自动补全（Completion）

**触发字符**：`.`（实例引脚访问）、`:`（模块访问）、`[`（端口切片）、` `（关键字）

**LSP**：
- `textDocument/completion` → `Vec<CompletionItem>`
- `completionItem/resolve` → 详细信息（可选）

**候选来源**：
| 上下文 | 候选 |
|---|---|
| `component` 后 | 系统组件库 + 项目组件 + 接口 |
| `interface` 后 | 系统接口库 + 项目接口 |
| `<name>.` | 该实例的引脚列表（从 `McPins` 取） |
| `<name>:` | 该 name 的属性列表 |
| 任意位置 | 关键字（`component`/`interface`/`enum`/`module`/`func`） |
| `func` 后 | 用户已定义函数名 + 系统函数 |

**详见** [features/completion.md](./features/completion.md)

---

### 2.6 自动格式化（Formatting）

**行为**：美化 mcode 源码，统一缩进、换行、对齐。

**LSP**：
- `textDocument/formatting` → `Vec<TextEdit>`
- `textDocument/rangeFormatting` → `Vec<TextEdit>`

**格式策略**（建议）：
- 缩进：4 空格（可配置）
- `component X {` 后换行 + 缩进
- 运算符 `-`, `+`, `->`, `=>` 两侧空格
- `pin` 块对齐 `=`、`name`
- `pins = [...]` 内每个 pin 独立一行
- 行长度上限 100（可配置）

**触发**：保存时自动格式化（settings.json: `"[mcode]": { "editor.formatOnSave": true }`）

**详见** [features/formatting.md](./features/formatting.md)

---

### 2.7 内嵌提示（Inlay Hints）

**状态**：❌ 未实现——**声明已撤**（provider 曾声明但恒回空数组，是空转的承诺面）。

**它是什么**：在源码里直接显示推断信息，例如 `func foo(x, y)` 后显示 `-> ReturnType`、实例引用 `u1` 后显示 `: ComponentName`。

**为何未做**：推断要有类型面。当前内核没有供插件消费的稳定类型面，在没有真值来源时给不出诚实的提示。

**去向**：内核提供类型面后重新立面并实装；在此之前插件不声明该能力（`mcodels.inlayHintsEnabled` 配置项一并撤除）。

**详见** [features/inlay-hints.md](./features/inlay-hints.md)

---

### 2.8 悬停文档（Hover）

**行为**：鼠标悬停符号时显示信息气泡。

**LSP**：
- `textDocument/hover` → `Hover { contents, range }`

**气泡内容**（按符号类型）：
| 符号 | 内容 |
|---|---|
| 组件名 | 该组件的注释 / spec / 引脚列表 / 默认值 |
| 接口名 | 接口成员列表 |
| 枚举 | 所有枚举值 |
| 函数 | 签名 + 注释 |
| 引脚 | 引脚号 + 名字 + 类型（in/out/io） |
| 实例引用 | 实例类型 + 引脚映射 |

**详见** [features/hover.md](./features/hover.md)

---

### 2.9 重命名符号（Rename）

**行为**：F2 改名实例 / 组件 / 接口名，所有引用同步。

**LSP**：
- `textDocument/rename` → `WorkspaceEdit`
- `textDocument/prepareRename` → `Range | null`（P2）

**约束**：
- 内置类型（`RES`/`CAP`/`DIO` 等）**不能**重命名
- 跨文件引用必须全部更新

**详见** [features/rename.md](./features/rename.md)

---

### 2.10 文档大纲（Document Outline）

**行为**：VSCode 左侧 Explorer 显示当前文件结构。

**LSP**：
- `textDocument/documentSymbol` → `Vec<SymbolInformation>` 或 `Vec<DocumentSymbol>`

**层级**：
```
▼ Component: my_basic
   Pins: VCC, GND, IN, OUT
   Attributes: Vgs, Rdson
▶ Interface: my_iface
▶ Enum: my_enum
▶ Function: helper
```

**详见** [features/document-symbol.md](./features/document-symbol.md)

---

### 2.11 注释切换（Comment Toggle）

**行为**：Ctrl+/ 切换行注释，Shift+Alt+A 切换块注释。

**实现**：VSCode language config（不需要 LSP server）。

**关键配置**（`language-configuration.json`）：
```json
{
  "comments": {
    "lineComment": "//",
    "blockComment": ["/*", "*/"]
  }
}
```

**详见** [features/comment-toggle.md](./features/comment-toggle.md)

---

### 2.12 项目级符号搜索（Workspace Symbol）

**行为**：Ctrl+T 跨文件搜索符号。

**LSP**：
- `workspace/symbol` → `Vec<SymbolInformation>`

**数据源**：项目索引 `ProjectIndex::symbol_to_loc`

**详见** [features/workspace-symbol.md](./features/workspace-symbol.md)

---

### 2.13 设置项（Settings）

**`package.json` 必填配置**：

| 配置 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `mcodels.systemRoot` | `string \| null` | `null` | 系统库根目录 |
| `mcodels.projectRoot` | `string \| null` | `null` | 项目根目录 |
| `mcodels.trace.server` | `string` | `"off"` | `"off" / "messages" / "verbose"` |
| `mcodels.diagnosticsDebounceMs` | `number` | `150` | 诊断防抖 |
| `mcodels.semanticTokensEnabled` | `boolean` | `true` | 语义高亮开关 |
| `mcodels.formatTabSize` | `number` | `4` | 格式化缩进 |
| `mcodels.formatInsertFinalNewline` | `boolean` | `true` | 文件末尾换行 |

**详见** [features/settings.md](./features/settings.md)

---

### 2.14 命令（Commands）

**`package.json` 必填命令**：

| 命令 ID | 标题 | 快捷键 |
|---|---|---|
| `mcode.build` | `Mcode: Build Netlist` | — |
| `mcode.checkFile` | `Mcode: Check File` | — |
| `mcode.explainCode` | `Mcode: Explain Code` | — |
| `mcode.previewViz` | `Mcode: Preview Netlist` | — |
| `mcode.toggleLineComment` | `Mcode: Toggle Line Comment` | Ctrl+/ |

**详见** [features/commands.md](./features/commands.md)

---

### 2.15 状态栏指示（Status Bar）

**行为**：右下角状态栏显示 LSP server 状态（客户端 `StatusBarItem`，`StatusBarAlignment.Right`）。

状态随 server 生命周期与解析进度变化；构建期的告警/错误另走 `MCode Build` 输出通道与专属 DiagnosticCollection，不覆盖逐文件诊断。

**详见** [features/status-bar.md](./features/status-bar.md)

---

## 3. 验收清单（Definition of Done）

每个功能必须满足：

1. ✅ LSP 协议层正确（spec 一致）
2. ✅ 后端实现对应 mcc 接口（不 hack）
3. ✅ 至少 3 个单测（happy / not-found / 边界）
4. ✅ 至少 1 个 E2E 测试（vscode-test）
5. ✅ 配置项（如有）写入 `package.json` 并有描述
6. ✅ 命令注册（如有）
7. ✅ 用户文档（`features/X.md`）

---

## 4. 矩阵的来历与维护律

本矩阵从 `work.md` §1 的 7 项起步，扩展为 38 项并逐行与实装对账。

**维护律**（U738 裁定）：

1. 本表是唯一实现真源——README 与 roadmap 从本表派生，不另立说法。
2. 状态以实装为准：对不上的是文档错，改文档。
3. 声明了却空转的能力最伤信任：要么实装，要么从能力面撤下（不留半成品）。
4. 新增能力先入矩阵（连同验收锚），再落码。
5. 状态与实装不符且来不及修时，先撤声明、再排期——不留"承诺了却空转"的行。
