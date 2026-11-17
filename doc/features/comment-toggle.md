# 12. 注释切换（Comment Toggle）

## 12.1 功能目标

VSCode 的两条标准命令：

- **Ctrl+/** —— 切换行注释（`//`）
- **Shift+Alt+A** —— 切换块注释（`/* ... */`）

## 12.2 实现方式

**两种实现路径**：

### 路径 A：VSCode Language Configuration（推荐）

VSCode 内置支持，无需 LSP server。配置 `contributes.languages`：

```json
{
  "contributes": {
    "languages": [
      {
        "id": "mcode",
        "extensions": [".mc"],
        "comments": {
          "lineComment": "//",
          "blockComment": ["/*", "*/"]
        },
        "configuration": "./language-configuration.json"
      }
    ]
  }
}
```

VSCode 自带的 `editor.action.commentLine` / `editor.action.blockComment` 自动识别。

### 路径 B：自定义 LSP handler（更灵活，可感知上下文）

如果需要根据 token 类型智能判断（不注释字符串内的 `//`，不注释条件编译块），实现：

```rust
async fn toggle_line_comment(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
    let uri = params.text_document.uri;
    let rope = state.rope(&uri)?;
    let ranges = params.text_document.uri;  // 由客户端传入的当前选区
    
    // 1. 用 mcc token 流识别每行"代码" vs "注释" vs "字符串内"
    // 2. 决定哪些行要 toggle
    // 3. 构造 TextEdit
}
```

## 12.3 路径 A 的 `language-configuration.json`

```json
{
  "comments": {
    "lineComment": "//",
    "blockComment": ["/*", "*/"]
  },
  "brackets": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"]
  ],
  "autoClosingPairs": [
    { "open": "{", "close": "}" },
    { "open": "[", "close": "]" },
    { "open": "(", "close": ")" },
    { "open": "\"", "close": "\"", "notIn": ["string"] }
  ],
  "surroundingPairs": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
    ["\"", "\""]
  ],
  "indentationRules": {
    "increaseIndentPattern": "^.*\\{[^}\"']*$|^.*\\[[^\\]\"']*$",
    "decreaseIndentPattern": "^\\s*\\}|^\\s*\\]"
  },
  "folding": {
    "markers": {
      "start": "^\\s*(component|interface|enum|func|module)\\s+\\w+",
      "end": "^\\s*\\}"
    }
  },
  "wordPattern": "[A-Za-z_][A-Za-z0-9_]*",
  "onEnterRules": [
    {
      "beforeText": "^\\s*(component|interface|enum|func|module)\\s+.*\\{$",
      "afterText": "^\\s*\\}$",
      "indent": "indent"
    }
  ]
}
```

## 12.4 路径 B 实现细节

如果走路径 B（自定义 handler），需要：

```rust
// features::comment_toggle::toggle_line
pub fn toggle_line(
    rope: &Rope,
    line_indices: &[usize],  // 要处理的行索引
) -> Vec<TextEdit> {
    // 1. 取每行的内容
    // 2. 检测是否已是注释（trim 后以 // 开头）
    // 3. 已注释 → 删 // 前缀；未注释 → 加 // 前缀
    // 4. 注意：单行内嵌注释 `code // ...` 不 toggle
}
```

## 12.5 测试

### 路径 A：自动由 VSCode 测试覆盖
- 选中代码 → Ctrl+/ → 注释出现 `//`
- 再次 Ctrl+/ → 注释消失

### 路径 B 单测（如果实现）
1. 单行 `let x = 1;` → toggle 后 `// let x = 1;`
2. 多行选中 → 每行都加 `//`
3. 已注释行 → toggle 后移除 `//`
4. 块注释 `/* ... */` 单独一行 → toggle 行注释不影响
5. 字符串 `"// not comment"` → 不注释

## 12.6 当前状态

- ✅ 行注释切换已实现：`language-configuration.json` 的 `comments` 段 ＋ 客户端命令 `mcode.toggleLineComment`（认 `//` 与 `#`）
- ❌ 块注释切换未实现：没有对应命令（`comments.blockComment` 已配，但没有切换命令）
## 12.7 重构工作（推荐路径 A）

1. 创建 `language-configuration.json`（含 comments / brackets / autoClosingPairs / indentRules / folding markers / onEnterRules）
2. 在 `package.json` 的 `contributes.languages` 中注册
3. 在 `package.json` 的 `contributes.commands` 注册自定义命令：
   ```json
   {
     "command": "mcode.toggleLineComment",
     "title": "Mcode: Toggle Line Comment",
     "category": "Mcode",
     "keybindings": [{ "command": "mcode.toggleLineComment", "key": "ctrl+slash", "when": "editorTextFocus && editorLangId == mcode" }]
   }
   ```
4. 在 `client/src/extension.ts` 注册命令（用 `vscode.commands.registerCommand` + `vscode.commands.executeCommand('editor.action.commentLine')`）

如果选择路径 B：额外加 `features::comment_toggle.rs` 模块 + LSP handler。

## 12.8 键绑定

```json
{
  "keybindings": [
    {
      "command": "mcode.toggleLineComment",
      "key": "ctrl+/",
      "when": "editorTextFocus && editorLangId == mcode"
    },
    {
      "command": "mcode.toggleBlockComment",
      "key": "shift+alt+a",
      "when": "editorTextFocus && editorLangId == mcode"
    }
  ]
}
```