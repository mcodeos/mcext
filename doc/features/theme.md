# 33. 主题（Theme）

## 33.1 目标

提供 mcode 语义令牌的默认颜色，避免用户必须自己写 `settings.json` 配色。

## 33.2 package.json 注册

```json
{
  "contributes": {
    "semanticTokenScopes": [
      {
        "language": "mcode",
        "scopes": {
          "variable.readonly": "variable.other.readonly",
          "variable.declaration": "variable.other.declaration",
          "class": "entity.name.class",
          "class.declaration": "entity.name.class.declaration",
          "interface": "entity.name.interface",
          "interface.declaration": "entity.name.interface.declaration",
          "enum": "entity.name.enum",
          "enum.declaration": "entity.name.enum.declaration",
          "function": "entity.name.function",
          "function.declaration": "entity.name.function.declaration",
          "parameter": "variable.parameter",
          "property": "variable.other.property",
          "keyword": "keyword.control.mcode",
          "operator": "keyword.operator.mcode",
          "string": "string.quoted.double.mcode",
          "number": "constant.numeric.mcode",
          "comment": "comment.line.double-slash.mcode"
        }
      }
    ],
    "semanticTokenColors": {
      "variable.readonly": "#4FC1FF",
      "variable.declaration": "#9CDCFE",
      "class": "#4EC9B0",
      "class.declaration": "#4EC9B0",
      "interface": "#B5CEA8",
      "interface.declaration": "#B5CEA8",
      "enum": "#FFD700",
      "enum.declaration": "#FFD700",
      "function": "#DCDCAA",
      "function.declaration": "#DCDCAA",
      "parameter": "#9CDCFE",
      "property": "#9CDCFE",
      "keyword": "#569CD6",
      "operator": "#D4D4D4",
      "string": "#CE9178",
      "number": "#B5CEA8",
      "comment": "#6A9955"
    }
  }
}
```

## 33.2.1 颜色方案选择

| 风格 | 配色来源 |
|---|---|
| 浅色主题 | 参考 VSCode Light+ 主题 |
| 深色主题 | 参考 VSCode Dark+ 主题（默认） |

## 33.3 contribution themes

如果提供整个主题（不推荐，scope 太大）：

```json
{
  "contributes": {
    "themes": [
      {
        "label": "Mcode Dark",
        "uiTheme": "vs-dark",
        "path": "./themes/mcode-dark.json"
      }
    ]
  }
}
```

**建议只做 semanticTokenColors**（范围小、不影响用户已有主题）。

## 33.4 当前状态

- ✅ 已实现：`contributes.semanticTokenScopes`（mcode）＋ 颜色定义

## 33.5 重构工作

1. 在 `package.json` 写入 semanticTokenColors
2. 测试：切换主题，验证 mcode 符号颜色变化
3. 文档：让用户知道在哪里改颜色