# 29. 命令（Commands）

## 29.1 package.json 注册

```json
{
  "contributes": {
    "commands": [
      {
        "command": "mcode.restartServer",
        "title": "Mcode: Restart Language Server",
        "category": "Mcode"
      },
      {
        "command": "mcode.buildProject",
        "title": "Mcode: Build Netlist",
        "category": "Mcode"
      },
      {
        "command": "mcode.parseCurrentFile",
        "title": "Mcode: Parse Current File",
        "category": "Mcode"
      },
      {
        "command": "mcode.showOutput",
        "title": "Mcode: Show Output",
        "category": "Mcode"
      },
      {
        "command": "mcode.toggleInlayHints",
        "title": "Mcode: Toggle Inlay Hints",
        "category": "Mcode"
      },
      {
        "command": "mcode.openProjectOutline",
        "title": "Mcode: Open Project Outline",
        "category": "Mcode"
      },
      {
        "command": "mcode.formatDocument",
        "title": "Mcode: Format Document",
        "category": "Mcode"
      },
      {
        "command": "mcode.toggleLineComment",
        "title": "Mcode: Toggle Line Comment",
        "category": "Mcode"
      },
      {
        "command": "mcode.toggleBlockComment",
        "title": "Mcode: Toggle Block Comment",
        "category": "Mcode"
      },
      {
        "command": "mcode.gotoNextError",
        "title": "Mcode: Go to Next Error",
        "category": "Mcode"
      },
      {
        "command": "mcode.gotoPrevError",
        "title": "Mcode: Go to Previous Error",
        "category": "Mcode"
      }
    ]
  }
}
```

## 29.2 client/src/extension.ts 注册

```typescript
const commands: Array<[string, (...args: any[]) => any]> = [
  ["mcode.restartServer", () => client.restart()],
  ["mcode.showOutput", () => traceOutputChannel.show()],
  ["mcode.toggleInlayHints", () => {
    const config = vscode.workspace.getConfiguration("mcodels");
    const cur = config.get<boolean>("inlayHints.enabled", true);
    config.update("inlayHints.enabled", !cur, true);
  }],
  ["mcode.formatDocument", () => {
    vscode.commands.executeCommand("editor.action.formatDocument");
  }],
  ["mcode.toggleLineComment", () => {
    vscode.commands.executeCommand("editor.action.commentLine");
  }],
  ["mcode.toggleBlockComment", () => {
    vscode.commands.executeCommand("editor.action.blockComment");
  }],
  ["mcode.gotoNextError", () => {
    vscode.commands.executeCommand("editor.action.marker.next");
  }],
  ["mcode.gotoPrevError", () => {
    vscode.commands.executeCommand("editor.action.marker.prev");
  }],
  ["mcode.buildProject", async () => {
    const result = await client.sendRequest("workspace/executeCommand", {
      command: "mcode.buildProject",
      arguments: [],
    });
    vscode.window.showInformationMessage(`Build: ${JSON.stringify(result)}`);
  }],
  ["mcode.parseCurrentFile", async () => {
    if (vscode.window.activeTextEditor) {
      await client.sendNotification("workspace/didChangeWatchedFiles", {
        changes: [{
          uri: vscode.window.activeTextEditor.document.uri,
          type: FileChangeType.Changed,
        }],
      });
    }
  }],
];

for (const [cmd, handler] of commands) {
  context.subscriptions.push(
    vscode.commands.registerCommand(cmd, handler)
  );
}
```

## 29.3 服务端 executeCommand

```rust
// 在 ServerCapabilities 中：
execute_command_provider: Some(ExecuteCommandOptions {
    commands: vec![
        "mcode.buildProject".to_string(),
        "mcode.parseCurrentFile".to_string(),
    ],
    work_done_progress_options: Default::default(),
}),

// handler
async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
    match params.command.as_str() {
        "mcode.buildProject" => {
            self.client.log_message(MessageType::INFO, "Building netlist...").await;
            let result = self.state.lock().await.build_project().await?;
            Ok(Some(serde_json::to_value(result)?))
        }
        "mcode.parseCurrentFile" => {
            // 触发当前文件重解析
            if let Some(uri) = params.arguments.first().and_then(|v| v.as_str()) {
                let url = Url::parse(uri)?;
                self.state.lock().await.schedule_reparse(url, -1).await;
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}
```

## 29.4 当前状态

- ✅ 已实现：5 个真实命令（`mcode.build` / `mcode.checkFile` / `mcode.explainCode` / `mcode.previewViz` / `mcode.toggleLineComment`）；旧的 `dummy.do_something` 占位已撤

## 29.5 重构工作

1. 在 `package.json` 写入完整命令清单
2. 在 `client/src/extension.ts` 注册
3. 在 server 端实现 `execute_command` handler
4. 加 E2E 测试