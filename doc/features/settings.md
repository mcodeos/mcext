# 31. 设置项（Settings）

完整的 `package.json::contributes.configuration`。

## 31.1 核心配置

```json
{
  "mcodels.systemRoot": {
    "type": ["string", "null"],
    "default": null,
    "description": "Path to MC system library root (sets MCC_SYSTEM_ROOT). Falls back to: env var → ${workspaceFolder}/mc → ~/.mcode",
    "scope": "window"
  },
  "mcodels.projectRoot": {
    "type": ["string", "null"],
    "default": null,
    "description": "Path to MC project root. Defaults to first workspace folder.",
    "scope": "window"
  },
  "mcodels.trace.server": {
    "type": "string",
    "enum": ["off", "messages", "verbose"],
    "enumDescriptions": [
      "No traces",
      "Error only",
      "Full log"
    ],
    "default": "off",
    "description": "Traces the communication between VS Code and the language server.",
    "scope": "window"
  }
}
```

## 31.2 功能开关

```json
{
  "mcodels.semanticTokens.enabled": { "type": "boolean", "default": true },
  "mcodels.inlayHints.enabled": { "type": "boolean", "default": true },
  "mcodels.diagnostics.enabled": { "type": "boolean", "default": true },
  "mcodels.completion.enabled": { "type": "boolean", "default": true },
  "mcodels.hover.enabled": { "type": "boolean", "default": true },
  "mcodels.format.enabled": { "type": "boolean", "default": true }
}
```

## 31.3 性能调优

```json
{
  "mcodels.diagnostics.debounceMs": {
    "type": "number",
    "default": 150,
    "minimum": 0,
    "maximum": 5000,
    "description": "Debounce duration for re-parsing (ms)"
  },
  "mcodels.index.debounceMs": {
    "type": "number",
    "default": 500,
    "description": "Debounce duration for project re-indexing (ms)"
  },
  "mcodels.perf.maxFileSizeKB": {
    "type": "number",
    "default": 1024,
    "description": "Skip parsing files larger than this"
  }
}
```

## 31.4 格式化

```json
{
  "mcodels.format.tabSize": { "type": "number", "default": 4 },
  "mcodels.format.insertSpaces": { "type": "boolean", "default": true },
  "mcodels.format.maxLineLength": { "type": "number", "default": 100 },
  "mcodels.format.insertFinalNewline": { "type": "boolean", "default": true },
  "mcodels.format.trimTrailingWhitespace": { "type": "boolean", "default": true },
  "mcodels.format.alignAssignments": { "type": "boolean", "default": false }
}
```

## 31.5 补全

```json
{
  "mcodels.completion.triggerChars": {
    "type": "array",
    "items": { "type": "string" },
    "default": [".", ":", "["]
  },
  "mcodels.completion.enableSnippets": { "type": "boolean", "default": true },
  "mcodels.completion.enableSystemLib": { "type": "boolean", "default": true },
  "mcodels.completion.maxItems": { "type": "number", "default": 100 }
}
```

## 31.6 内嵌提示细节

```json
{
  "mcodels.inlayHints.showInstanceType": { "type": "boolean", "default": true },
  "mcodels.inlayHints.showPinNumber": { "type": "boolean", "default": true },
  "mcodels.inlayHints.showSliceExpansion": { "type": "boolean", "default": false },
  "mcodels.inlayHints.suppressWhenTypeMatchesName": { "type": "boolean", "default": true }
}
```

## 31.7 错误处理

```json
{
  "mcodels.diagnostics.openOutputOnError": { "type": "boolean", "default": true },
  "mcodels.diagnostics.pullMode": { "type": "boolean", "default": false }
}
```

## 31.8 当前状态

- ✅ 已实现：6 项配置（`systemRoot` / `projectRoot` / `semanticTokensEnabled` / `diagnosticsDebounceMs` / `formatTabSize` / `formatInsertFinalNewline`）

## 31.9 重构工作

1. 在 `package.json` 写入完整配置
2. 在 `Backend::initialize` 读取并应用 `systemRoot` / `projectRoot`
3. 在每个 feature 模块读取自己的开关
4. 加单元测试覆盖配置加载