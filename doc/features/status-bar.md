# 28. 状态栏指示（Status Bar）

## 28.1 目标

VSCode 右下角显示 LSP server 状态和当前文件错误数。

## 28.2 实现

纯客户端实现，用 `vscode.window.createStatusBarItem()`。

## 28.3 client/src/status.ts

```typescript
import * as vscode from "vscode";

export class McodeStatusBar {
    private item: vscode.StatusBarItem;
    private currentState: "idle" | "parsing" | "error" | "indexed" = "idle";
    
    constructor() {
        this.item = vscode.window.createStatusBarItem(
            vscode.StatusBarAlignment.Right,
            100
        );
        this.item.command = "mcode.showOutput";
        this.item.tooltip = "Mcode Language Server";
        this.update();
    }
    
    setState(state: typeof this.currentState, info?: { errors?: number; files?: number }) {
        this.currentState = state;
        this._extraInfo = info;
        this.update();
    }
    
    private _extraInfo: { errors?: number; files?: number } = {};
    
    private update() {
        switch (this.currentState) {
            case "idle":
                this.item.text = "$(check) mcode";
                this.item.backgroundColor = undefined;
                break;
            case "parsing":
                this.item.text = "$(sync~spin) mcode parsing…";
                this.item.backgroundColor = new vscode.ThemeColor(
                    "statusBarItem.warningBackground"
                );
                break;
            case "error":
                this.item.text = `$(error) mcode: ${this._extraInfo.errors ?? 0} errors`;
                this.item.backgroundColor = new vscode.ThemeColor(
                    "statusBarItem.errorBackground"
                );
                break;
            case "indexed":
                this.item.text = `$(database) mcode: ${this._extraInfo.files ?? 0} files`;
                this.item.backgroundColor = undefined;
                break;
        }
        this.item.show();
    }
    
    dispose() {
        this.item.dispose();
    }
}
```

## 28.4 触发点

```typescript
// client/src/extension.ts
const status = new McodeStatusBar();
context.subscriptions.push(status);

// 1. 文件变更 → parsing
workspace.onDidChangeTextDocument(() => status.setState("parsing"));

// 2. 收到 diagnostics → error / idle
client.onNotification("$/mcc/diagnostics", (params: { uri: string; count: number }) => {
    if (params.count > 0) {
        status.setState("error", { errors: params.count });
    } else {
        status.setState("idle");
    }
});

// 3. 索引完成 → indexed
client.onNotification("$/mcc/indexDone", (params: { files: number }) => {
    status.setState("indexed", { files: params.files });
});
```

需要在 server 端加 `$/progress` 或自定义通知：

```rust
// server/src/features/notify.rs
pub async fn notify_diagnostics(client: &Client, uri: &Url, count: usize) {
    client.send_notification("$/mcc/diagnostics", json!({
        "uri": uri.to_string(),
        "count": count,
    })).await;
}

pub async fn notify_index_done(client: &Client, files: usize) {
    client.send_notification("$/mcc/indexDone", json!({ "files": files })).await;
}
```

## 28.5 当前状态

- ✅ 已实现：客户端 `StatusBarItem`（右侧），随 server 生命周期与解析进度变化

## 28.6 重构工作

1. 实现 `client/src/status.ts`
2. 在 server 端加自定义通知
3. 在 `client/src/extension.ts` 集成
4. 加 E2E 测试：故意写错 → 状态栏显示错误数