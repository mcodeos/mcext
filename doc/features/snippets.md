# 18. 代码片段（Snippets）

## 18.1 功能目标

VSCode 内置 snippet 引擎，输入前缀 + Tab 自动展开为模板。

## 18.2 实现方式

不依赖 LSP server —— 直接在 `package.json` 注册 `contributes.snippets`。

## 18.3 snippets/mcode.json

```json
{
  "component": {
    "prefix": ["component", "comp"],
    "body": [
      "component ${1:name} {",
      "\t$0",
      "}"
    ],
    "description": "Create a new component"
  },
  "interface": {
    "prefix": ["interface", "iface"],
    "body": [
      "interface ${1:name} {",
      "\t$0",
      "}"
    ],
    "description": "Create a new interface"
  },
  "enum": {
    "prefix": "enum",
    "body": [
      "enum ${1:name} {",
      "\t$2 = ${3:value}",
      "}"
    ],
    "description": "Create a new enum"
  },
  "func": {
    "prefix": "func",
    "body": [
      "func ${1:name}($2) {",
      "\t$0",
      "}"
    ],
    "description": "Create a new function"
  },
  "pin-io": {
    "prefix": "io",
    "body": "io ${1:1} = ${2:PIN_NAME}",
    "description": "Add an IO pin"
  },
  "pin-list": {
    "prefix": "pins",
    "body": [
      "pins = [",
      "\t$0",
      "]"
    ],
    "description": "Pin list block"
  },
  "instance": {
    "prefix": ["instance", "inst"],
    "body": "${1:name} = ${2:ComponentName}::${3:Type}($4)",
    "description": "Create a component instance"
  },
  "connection": {
    "prefix": ["conn", "wire"],
    "body": "${1:A} - ${2:B}",
    "description": "Series connection"
  },
  "parallel": {
    "prefix": ["parallel", "par"],
    "body": "(${1:A} + ${2:B})",
    "description": "Parallel connection"
  },
  "ground": {
    "prefix": "gnd",
    "body": "${1:net} -> GND",
    "description": "Connect to GND"
  },
  "power": {
    "prefix": ["vcc", "vdd"],
    "body": "${1:net} -> VCC",
    "description": "Connect to VCC"
  }
}
```

## 18.4 package.json 注册

```json
{
  "contributes": {
    "snippets": [
      {
        "language": "mcode",
        "path": "./snippets/mcode.json"
      }
    ]
  }
}
```

## 18.5 占位符语法

| 语法 | 含义 |
|---|---|
| `${1:name}` | 第 1 个光标停留位置，带默认名 |
| `${1\|a,b,c\|}` | 第 1 个位置，可选 a/b/c |
| `$0` | 最终光标位置 |
| `$TM_FILENAME` | 当前文件名 |
| `${TM_FILENAME/(.*)/${1:/upcase}/}` | 正则替换 |

## 18.6 当前状态

- ✅ 已实现：`contributes.snippets` 指向 `snippets/mcode.json`

## 18.7 重构工作

1. 创建 `snippets/mcode.json`
2. 在 `package.json` 注册
3. 测试：用编辑器手动验证每个 snippet