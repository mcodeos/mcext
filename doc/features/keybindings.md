# 32. 键绑定（Keybindings）

## 32.1 package.json 注册

```json
{
  "contributes": {
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
      },
      {
        "command": "mcode.formatDocument",
        "key": "shift+alt+f",
        "when": "editorTextFocus && editorLangId == mcode"
      },
      {
        "command": "mcode.restartServer",
        "key": "ctrl+shift+p mcode.restartServer",
        "when": "editorTextFocus"
      },
      {
        "command": "mcode.buildProject",
        "key": "ctrl+shift+b",
        "when": "editorTextFocus && editorLangId == mcode"
      },
      {
        "command": "mcode.gotoNextError",
        "key": "f8",
        "when": "editorTextFocus && editorLangId == mcode"
      },
      {
        "command": "mcode.gotoPrevError",
        "key": "shift+f8",
        "when": "editorTextFocus && editorLangId == mcode"
      },
      {
        "command": "mcode.toggleInlayHints",
        "key": "ctrl+alt+i",
        "when": "editorTextFocus && editorLangId == mcode"
      },
      {
        "command": "mcode.openProjectOutline",
        "key": "ctrl+shift+o",
        "when": "editorTextFocus && editorLangId == mcode"
      }
    ]
  }
}
```

## 32.2 when 子句

| 子句 | 含义 |
|---|---|
| `editorTextFocus` | 编辑器有焦点 |
| `editorLangId == mcode` | 当前文件是 mcode |
| `editorHasSelection` | 有选中内容 |
| `editorReadonly` | 文件只读 |
| `inSnippetMode` | 在 snippet 占位符中 |
| `config.mcodels.inlayHints.enabled` | 配置启用 |
| `mcode.serverRunning` | 自定义上下文：server 是否在跑 |

## 32.3 当前状态

- ✅ 已定义 5 个键绑定：`mcode.build` / `mcode.checkFile` / `mcode.explainCode` / `mcode.previewViz` / `mcode.toggleLineComment`

## 32.4 重构工作

1. 在 `package.json` 写入键绑定
2. 文档中说明常用快捷键
3. 测试：每个命令在指定快捷键下能触发