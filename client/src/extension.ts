/* --------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. See License.txt in the project root for license information.
 * ------------------------------------------------------------------------------------------ */

import * as fs from "fs";
import * as path from "path";

import {
  languages,
  workspace,
  EventEmitter,
  ExtensionContext,
  window,
  InlayHintsProvider,
  TextDocument,
  CancellationToken,
  Range,
  InlayHint,
  TextDocumentChangeEvent,
  ProviderResult,
  commands,
  WorkspaceEdit,
  TextEdit,
  Selection,
  Uri,
  ViewColumn,
  WebviewPanel,
} from "vscode";

import {
  Disposable,
  Executable,
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient;
let clientStarted: Promise<void> | undefined;
// type a = Parameters<>;

// Circuit preview state: the open panel + the project (nearest manifest dir)
// it currently renders, so we can auto-refresh on cross-project file switches.
let previewPanel: WebviewPanel | undefined;
let previewProjectId: string | null = null;
let previewRenderSeq = 0;

const PROJECT_MANIFEST_NAMES = ["project.toml", "manifest.toml", "mcc.toml"];

// Nearest ancestor directory containing a project manifest, else null.
// Mirrors the server's find_project_root_from_file so client & server agree
// (both require an actual file — a directory named `project.toml` doesn't count).
function projectIdFor(filePath: string): string | null {
  let dir = path.dirname(filePath);
  for (;;) {
    for (const name of PROJECT_MANIFEST_NAMES) {
      if (isFile(path.join(dir, name))) return dir;
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null; // reached filesystem root
    dir = parent;
  }
}

function isFile(p: string): boolean {
  try {
    return fs.statSync(p).isFile();
  } catch {
    return false;
  }
}

// "Same project" per design: both resolve to a manifest dir AND they match.
// A standalone file (null id) is its own project → never "same".
function sameProject(a: string | null, b: string | null): boolean {
  return a !== null && b !== null && a === b;
}

export async function activate(context: ExtensionContext) {

  const traceOutputChannel = window.createOutputChannel("MCode");
  const command = process.env.SERVER_PATH || "mcodels";
  const run: Executable = {
    command,
    options: {
      env: {
        ...process.env,
        // eslint-disable-next-line @typescript-eslint/naming-convention
        RUST_LOG: "debug",
      },
    },
  };
  const serverOptions: ServerOptions = {
    run,
    debug: run,
  };
  // If the extension is launched in debug mode then the debug server options are used
  // Otherwise the run options are used
  // Options to control the language client
  let clientOptions: LanguageClientOptions = {
    // Register the server for plain text documents
    documentSelector: [{ scheme: "file", language: "mcode" }],
    synchronize: {
      // Notify the server about file changes to '.clientrc files contained in the workspace
      fileEvents: workspace.createFileSystemWatcher("**/.clientrc"),
    },
    traceOutputChannel,
  };

  // Create the language client and start the client.
  client = new LanguageClient("mcode", "MCode", serverOptions, clientOptions);
  // activateInlayHints(context);
  clientStarted = client.start();

  // viz circuit preview: build + render the active .mc file into a webview.
  // Optional command argument = top module name for no-project files; when
  // omitted, mcc falls back to the first module of the file (usually `main`).
  context.subscriptions.push(
    commands.registerCommand("mcode.previewViz", (top?: string) => previewViz(top))
  );

  // Auto-refresh an already-open preview when the active .mc file switches to
  // a different project (by nearest project manifest). Same project → no
  // refresh. Does NOT auto-open a preview (only acts when one is tracked).
  context.subscriptions.push(
    window.onDidChangeActiveTextEditor((editor) => {
      if (!editor || editor.document.languageId !== "mcode") return;
      if (!previewPanel) return;
      const filePath = editor.document.uri.fsPath;
      const newProjectId = projectIdFor(filePath);
      if (sameProject(previewProjectId, newProjectId)) return;
      previewProjectId = newProjectId;
      void renderPreview(filePath);
    })
  );
}

async function previewViz(top?: string): Promise<void> {
  const editor = window.activeTextEditor;
  if (!editor || editor.document.languageId !== "mcode") {
    window.showInformationMessage(
      "MCode: open a .mc file to preview its circuit."
    );
    return;
  }

  const filePath = editor.document.uri.fsPath;

  // Reuse an existing panel instead of stacking duplicates on repeated invocations.
  if (!previewPanel) {
    previewPanel = window.createWebviewPanel(
      "mcodeViz",
      `Circuit: ${path.basename(filePath)}`,
      ViewColumn.Beside,
      { enableScripts: true }
    );
    previewPanel.onDidDispose(() => {
      previewPanel = undefined;
      previewProjectId = null;
    });
  }

  // Record the project BEFORE rendering: creating the panel focuses it, which
  // fires onDidChangeActiveTextEditor; setting the id first avoids a spurious
  // immediate re-render.
  previewProjectId = projectIdFor(filePath);
  await renderPreview(filePath, top);
}

// Shared by the previewViz command and the cross-project auto-refresh listener.
async function renderPreview(filePath: string, top?: string): Promise<void> {
  const seq = ++previewRenderSeq;
  const panel = previewPanel;
  if (!panel) return;

  panel.title = `Circuit: ${path.basename(filePath)}`;
  panel.webview.html = loadingHtml("Rendering circuit…");

  try {
    if (clientStarted) {
      await clientStarted;
    }
    // Drop stale renders and guard against a panel disposed mid-flight.
    if (seq !== previewRenderSeq || previewPanel !== panel) return;
    const result = (await client.sendRequest("workspace/executeCommand", {
      command: "mcode.viz",
      arguments: top ? [filePath, top] : [filePath],
    })) as { ok: boolean; html?: string; error?: string } | null;
    if (seq !== previewRenderSeq || previewPanel !== panel) return;
    panel.webview.html =
      result && result.ok && result.html
        ? result.html
        : errorHtml(result?.error ?? "build.viz returned no html");
  } catch (e) {
    if (seq !== previewRenderSeq || previewPanel !== panel) return;
    panel.webview.html = errorHtml(String(e));
  }
}

function loadingHtml(message: string): string {
  return `<!DOCTYPE html><html><body><p>${escapeHtml(message)}</p></body></html>`;
}

function errorHtml(message: string): string {
  return `<!DOCTYPE html><html><body><pre>${escapeHtml(message)}</pre></body></html>`;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}

export function activateInlayHints(ctx: ExtensionContext) {
  const maybeUpdater = {
    hintsProvider: null as Disposable | null,
    updateHintsEventEmitter: new EventEmitter<void>(),

    async onConfigChange() {
      this.dispose();

      const event = this.updateHintsEventEmitter.event;
      // this.hintsProvider = languages.registerInlayHintsProvider(
      //   { scheme: "file", language: "mcode" },
      //   // new (class implements InlayHintsProvider {
      //   //   onDidChangeInlayHints = event;
      //   //   resolveInlayHint(hint: InlayHint, token: CancellationToken): ProviderResult<InlayHint> {
      //   //     const ret = {
      //   //       label: hint.label,
      //   //       ...hint,
      //   //     };
      //   //     return ret;
      //   //   }
      //   //   async provideInlayHints(
      //   //     document: TextDocument,
      //   //     range: Range,
      //   //     token: CancellationToken
      //   //   ): Promise<InlayHint[]> {
      //   //     const hints = (await client
      //   //       .sendRequest("custom/inlay_hint", { path: document.uri.toString() })
      //   //       .catch(err => null)) as [number, number, string][];
      //   //     if (hints == null) {
      //   //       return [];
      //   //     } else {
      //   //       return hints.map(item => {
      //   //         const [start, end, label] = item;
      //   //         let startPosition = document.positionAt(start);
      //   //         let endPosition = document.positionAt(end);
      //   //         return {
      //   //           position: endPosition,
      //   //           paddingLeft: true,
      //   //           label: [
      //   //             {
      //   //               value: `${label}`,
      //   //               // location: {
      //   //               //   uri: document.uri,
      //   //               //   range: new Range(1, 0, 1, 0)
      //   //               // }
      //   //               command: {
      //   //                 title: "hello world",
      //   //                 command: "helloworld.helloWorld",
      //   //                 arguments: [document.uri],
      //   //               },
      //   //             },
      //   //           ],
      //   //         };
      //   //       });
      //   //     }
      //   //   }
      //   // })()
      // );
    },

    onDidChangeTextDocument({ contentChanges, document }: TextDocumentChangeEvent) {
      // debugger
      // this.updateHintsEventEmitter.fire();
    },

    dispose() {
      this.hintsProvider?.dispose();
      this.hintsProvider = null;
      this.updateHintsEventEmitter.dispose();
    },
  };

  workspace.onDidChangeConfiguration(maybeUpdater.onConfigChange, maybeUpdater, ctx.subscriptions);
  workspace.onDidChangeTextDocument(maybeUpdater.onDidChangeTextDocument, maybeUpdater, ctx.subscriptions);

  maybeUpdater.onConfigChange().catch(console.error);
}
