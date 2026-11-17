# 35. 构建入口（Build Netlist）

## 35.1 目标

用户命令 `Mcode: Build Netlist` → 触发 mcc Pass-2 → 生成网表 → 显示结果。

## 35.2 LSP 协议

- **能力**：`execute_command_provider`
- **请求**：`workspace/executeCommand` { command: "mcode.buildProject" }

## 35.3 实现要点

### 35.3.1 命令定义

```json
// package.json
{
  "command": "mcode.buildProject",
  "title": "Mcode: Build Netlist",
  "category": "Mcode",
  "keybindings": [
    { "command": "mcode.buildProject", "key": "ctrl+shift+b", "when": "editorLangId == mcode" }
  ]
}
```

### 35.3.2 服务端 handler

```rust
async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<Value>> {
    match params.command.as_str() {
        "mcode.buildProject" => {
            // 1. 通知 client 开始
            let token = self.client.progress_begin("Building netlist...").await?;
            
            // 2. 取项目入口
            let state = self.state.lock().await;
            let project_root = state.project_root.clone();
            drop(state);
            
            // 3. 调 mcc
            let result = tokio::task::spawn_blocking(move || {
                mcc::mcc_set_project_root(&project_root);
                let entry_uri = find_entry(&project_root)?;
                mcc::mcc_load_project(&entry_uri)?;
                let (tree, inst_table) = mcc::mcc_build_flat(
                    &McIds::from("__top__"),
                    &entry_uri,
                    1000,
                )?;
                Ok::<_, anyhow::Error>((tree, inst_table))
            }).await?;
            
            match result {
                Ok((tree, inst_table)) => {
                    self.client.progress_end(token, "Done").await?;
                    self.client.show_message(MessageType::INFO, 
                        format!("Built {} nets, {} instances", tree.net_count, inst_table.len())).await?;
                    
                    // 4. 写输出
                    let output_path = project_root.join("build/output.net");
                    tokio::fs::create_dir_all(output_path.parent().unwrap()).await?;
                    tokio::fs::write(&output_path, mcc::vector::build_mc_vec(&tree)?).await?;
                    
                    self.client.show_message(MessageType::INFO, 
                        format!("Netlist written to {}", output_path.display())).await?;
                    
                    Ok(Some(json!({
                        "nets": tree.net_count,
                        "instances": inst_table.len(),
                        "output": output_path.to_string_lossy(),
                    })))
                }
                Err(e) => {
                    self.client.progress_end(token, &format!("Failed: {}", e)).await?;
                    self.client.show_message(MessageType::ERROR, e.to_string()).await?;
                    Ok(None)
                }
            }
        }
        _ => Ok(None),
    }
}
```

### 35.3.3 客户端调用

```typescript
// 在 client/src/extension.ts
vscode.commands.registerCommand("mcode.buildProject", async () => {
    try {
        const result = await client.sendRequest<{nets: number, instances: number, output: string}>(
            "workspace/executeCommand",
            { command: "mcode.buildProject", arguments: [] }
        );
        if (result) {
            vscode.window.showInformationMessage(
                `Built ${result.nets} nets, ${result.instances} instances.\n` +
                `Output: ${result.output}`
            );
            // 打开输出文件
            vscode.workspace.openTextDocument(result.output).then(doc => {
                vscode.window.showTextDocument(doc);
            });
        }
    } catch (e) {
        vscode.window.showErrorMessage(`Build failed: ${e}`);
    }
});
```

## 35.4 进度显示

LSP 的 `$/progress` 协议：

```
client.progress_begin(title)  → 返回 token
client.progress_report(token, message, percent)
client.progress_end(token, message)
```

## 35.5 当前状态

- ✅ 已实现：命令 `mcode.build` 走 daemon 构建；构建报告写 `MCode Build` 输出通道，告警/错误进专属 DiagnosticCollection（不覆盖逐文件诊断）

## 35.6 重构工作

1. 服务端实现 `mcode.buildProject`
2. 客户端注册命令
3. 加 `$/progress` 通知
4. 加 E2E 测试（fixture 项目 + 验证网表生成）