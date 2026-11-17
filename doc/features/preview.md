# 37. 内嵌预览（Preview Netlist）

## 37.1 功能目标

在编辑器里并排看到网表原理图：编辑 `.mc` 源码后，预览随构建结果重新渲染，不必切到浏览器。

## 37.2 入口

- **命令**：`mcode.previewViz`（`Mcode: Preview Netlist`）
- **自定义编辑器**：`contributes.customEditors` 注册的只读预览视图（`mcode.viz` 视图类型）
- 源码：`mcext/client/src/extension.ts`（`CircuitPreviewProvider`）

## 37.3 当前实现

- `CircuitPreviewProvider` 实现 `CustomReadonlyEditorProvider`，每个源文件对应一个 webview 面板；面板按源 URI 归档在 `panels` 表里。
- 每次渲染带一个递增序号，迟到的 RPC 响应不会覆盖更新的渲染；面板关闭后安排的定时重渲染也会被取消（`timers` 表）。
- 标签页图标区分明暗两套变体（SVG 以图片绘制，不能用 `currentColor`）。
- 编辑 `.mc` 后自动打开/刷新预览（`autoOpenPreview`）；窗口重载后恢复的标签页走 `resolveCustomEditor` 延迟渲染——此时没有活动编辑器，不能靠 `window.activeTextEditor` 触发。

## 37.4 当前状态

✅ 已实现。

## 37.5 边界

- 预览内容来自构建/渲染接口；渲染失败时在构建输出通道报错。
- 顶模块可由命令参数覆盖（`vizTopByPath`），未指定时由 mcc 自行选择文件的首个 module。
