# 20/22/38. Language Configuration

## 20.1 目标

定义 mcode 的语言属性：注释符号、括号对、自动闭合、缩进规则。

VSCode 内置支持，无需 LSP server。

## 20.2 language-configuration.json

放在扩展根目录（与 `package.json` 同级）：

```json
{
  "$schema": "./node_modules/@types/vscode/language-configuration.schema.json",
  
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
    { "open": "\"", "close": "\"", "notIn": ["string"] },
    { "open": "/*", "close": " */", "notIn": ["comment"] }
  ],

  "surroundingPairs": [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
    ["\"", "\""]
  ],

  "wordPattern": "[A-Za-z_][A-Za-z0-9_]*",

  "indentationRules": {
    "increaseIndentPattern": "^.*\\{[^}\"']*$|^.*\\[[^\\]\"']*$|^\\s*(component|interface|enum|func|module)\\s+\\w+.*\\{$",
    "decreaseIndentPattern": "^\\s*\\}|;\\s*$"
  },

  "onEnterRules": [
    {
      "beforeText": "^\\s*(component|interface|enum|func|module)\\s+.*\\{$",
      "afterText": "^\\s*\\}$",
      "indent": "indent"
    }
  ],

  "folding": {
    "markers": {
      "start": "^\\s*(component|interface|enum|func|module)\\s+\\w+",
      "end": "^\\s*\\}"
    }
  }
}
```

## 20.3 package.json 注册

```json
{
  "contributes": {
    "languages": [
      {
        "id": "mcode",
        "extensions": [".mc"],
        "aliases": ["mcode", "MCode"],
        "configuration": "./language-configuration.json"
      }
    ]
  }
}
```

## 20.4 字段说明

| 字段 | 作用 |
|---|---|
| `comments.lineComment` | Ctrl+/ 用 |
| `comments.blockComment` | Shift+Alt+A 用 |
| `brackets` | Ctrl+] 跳转匹配括号 |
| `autoClosingPairs` | 输入 `{` 自动加 `}` |
| `surroundingPairs` | 选中代码 + `"` 自动包 |
| `wordPattern` | 双击选择单词的边界 |
| `indentationRules` | 自动缩进 |
| `onEnterRules` | Enter 后自动缩进 |
| `folding.markers` | 代码折叠范围 |

## 20.5 当前状态

- ✅ 已实现：`language-configuration.json` 提供 `comments` / `brackets` / `autoClosingPairs` / `surroundingPairs` / `indentationRules` / `formatOnPaste`

## 20.6 重构工作

1. 创建 `language-configuration.json`
2. 在 `package.json` 注册
3. 测试：
   - 输入 `{` → 自动闭合 `}`
   - 输入 `component X {` → Enter 后自动缩进
   - 输入 `// xxx` → 行注释
   - 双击 `R442` → 选中整个标识符

## 38. 字符串/字符选择（Word Selection）

**行为**：双击标识符选中整个标识符（`R442`），双击引号内选中整个字符串。

**实现方式**：`language-configuration.json` 的 `wordPattern`。

**当前状态**：❌ 未实现——`language-configuration.json` 未定义 `wordPattern`，双击按 VSCode 默认的单词边界走。

**边界**：字符串字面量由 VSCode 内置的 word selection rules 处理（基于引号字符），不需要自定义。
