# 19. 语法高亮 - TextMate 兜底

## 19.1 目标

即使 LSP server 未启动，也能给 mcode 一个**基本**的语法高亮（关键字、字符串、数字、注释）。

## 19.2 实现

在 `package.json` 注册 TextMate 语法：

```json
{
  "contributes": {
    "grammars": [
      {
        "language": "mcode",
        "scopeName": "source.mcode",
        "path": "./syntaxes/mcode.tmLanguage.json"
      }
    ]
  }
}
```

## 19.3 syntaxes/mcode.tmLanguage.json

```json
{
  "$schema": "https://raw.githubusercontent.com/martinring/tmlanguage/master/tmlanguage.json",
  "name": "mcode",
  "scopeName": "source.mcode",
  "fileTypes": ["mc"],
  "patterns": [
    { "include": "#comments" },
    { "include": "#strings" },
    { "include": "#numbers" },
    { "include": "#keywords" },
    { "include": "#operators" },
    { "include": "#identifiers" }
  ],
  "repository": {
    "comments": {
      "patterns": [
        {
          "name": "comment.line.double-slash.mcode",
          "match": "//.*$"
        },
        {
          "name": "comment.block.mcode",
          "begin": "/\\*",
          "end": "\\*/"
        }
      ]
    },
    "strings": {
      "patterns": [
        {
          "name": "string.quoted.double.mcode",
          "begin": "\"",
          "end": "\"",
          "patterns": [
            { "name": "constant.character.escape.mcode", "match": "\\\\." }
          ]
        }
      ]
    },
    "numbers": {
      "patterns": [
        {
          "name": "constant.numeric.integer.mcode",
          "match": "\\b[0-9]+\\b"
        },
        {
          "name": "constant.numeric.float.mcode",
          "match": "\\b[0-9]+\\.[0-9]+\\b"
        },
        {
          "name": "constant.numeric.scientific.mcode",
          "match": "\\b[0-9]+(\\.[0-9]+)?[eE][+-]?[0-9]+\\b"
        },
        {
          "name": "constant.numeric.unit.mcode",
          "match": "\\b[0-9]+(m|k|M)?(Ω|Ω|F|H|V|A)\\b"
        }
      ]
    },
    "keywords": {
      "patterns": [
        {
          "name": "keyword.control.mcode",
          "match": "\\b(component|interface|enum|func|module|spec|layout|left|right|pins|io|in|out|inout)\\b"
        }
      ]
    },
    "operators": {
      "patterns": [
        {
          "name": "keyword.operator.mcode",
          "match": "(\\->|\\=>|\\+|\\-|\\=\\>|\\:\\:|\\.|\\[|\\])"
        }
      ]
    },
    "identifiers": {
      "patterns": [
        {
          "name": "variable.other.identifier.mcode",
          "match": "\\b[A-Za-z_][A-Za-z0-9_]*\\b"
        }
      ]
    }
  }
}
```

## 19.4 优先级

| 高亮来源 | 优先级 | 用途 |
|---|---|---|
| Semantic Tokens（来自 LSP server） | **高** | 真正的语义颜色 |
| TextMate 语法（兜底） | 低 | 没启 server 时也能看 |

VSCode 内部会把 Semantic Tokens 与 TextMate 叠加：TextMate 提供基础分类（关键字、字符串），Semantic Tokens 提供更精细的语义（区分用户组件 vs 系统组件）。

## 19.5 当前状态

- ✅ 已实现：`contributes.grammars` 指向 `syntaxes/` 下的 TextMate 语法（LSP 缺席时的高亮降级）

## 19.6 重构工作

1. 创建 `syntaxes/mcode.tmLanguage.json`
2. 在 `package.json` 注册
3. 测试：用纯文本编辑器打开 `.mc` 文件，看是否有基本高亮