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
  Diagnostic,
  DiagnosticSeverity,
  Position,
  ProgressLocation,
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

// Whole-project build state: the Output console the `mcc build` report is
// written to, and a dedicated DiagnosticCollection so build warnings/errors
// appear in the Problems tab WITHOUT clobbering the live per-file diagnostics
// published by the language server during editing.
const buildOutput = window.createOutputChannel("MCode Build");
const buildDiags = languages.createDiagnosticCollection("mcode-build");

// One diagnostic in the `mcode.buildProject` executeCommand response
// (flattened from the build.full pass0/pass1/pass2 envelope).
interface BuildDiag {
  phase: string;
  severity: string; // "error" | "warning" | "info" | "hint"
  code: number;
  message: string;
  file: string; // plain path or file:// URI
  line: number; // 1-based
  column: number; // 1-based
  pos: number;
  len: number;
}

interface BuildResult {
  ok: boolean;
  summary?: {
    module_count?: number;
    component_count?: number;
    interface_count?: number;
    instance_count?: number;
    net_count?: number;
    errors?: number;
    warnings?: number;
    elapsed_ms?: number;
    stats?: BuildStats;
  };
  diagnostics?: BuildDiag[];
  error?: string;
}

// Categorized build statistics — the same numbers `mcc build` prints in its
// Summary block: namespace classes and used classes each split into
// system (`/mcode/` library) vs project space, plus the instance breakdown.
interface BuildStats {
  ns_modules_system?: number;
  ns_modules_project?: number;
  ns_components_system?: number;
  ns_components_project?: number;
  ns_interfaces_system?: number;
  ns_interfaces_project?: number;
  used_modules_system?: number;
  used_modules_project?: number;
  used_components_system?: number;
  used_components_project?: number;
  module_insts?: number;
  component_insts?: number;
}

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

  // Whole-project build: run `mcc build` on the active file's project, write
  // the report to the "MCode Build" output channel and push warnings/errors
  // into the Problems tab.
  // NOTE: the client command is `mcode.build`, the server executeCommand is
  // `mcode.buildProject`. They must differ: vscode-languageclient auto-registers
  // every executeCommandProvider command (see ExecuteCommandFeature), so a
  // same-named client command would throw "command ... already exists".
  context.subscriptions.push(
    commands.registerCommand("mcode.build", () => buildProject())
  );
  context.subscriptions.push(buildOutput, buildDiags);

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

// Run a whole-project build via the language server (daemon `build.full` RPC),
// then render the report to the Output console and warnings/errors to the
// Problems tab.
async function buildProject(): Promise<void> {
  const editor = window.activeTextEditor;
  if (!editor || editor.document.languageId !== "mcode") {
    window.showInformationMessage(
      "MCode: open a .mc file to build its project."
    );
    return;
  }

  const filePath = editor.document.uri.fsPath;

  // Fresh build → drop the previous build's Problems entries.
  buildDiags.clear();

  await window.withProgress(
    { location: ProgressLocation.Window, title: "MCC: Building project…" },
    async () => {
      if (clientStarted) {
        await clientStarted;
      }
      try {
        const result = (await client.sendRequest("workspace/executeCommand", {
          // Server-side executeCommand id (advertised in executeCommandProvider).
          command: "mcode.buildProject",
          arguments: [filePath],
        })) as BuildResult | null;

        if (!result || !result.ok) {
          const msg = result?.error ?? "buildProject returned no result";
          buildOutput.appendLine(`[mcc build] FAILED: ${msg}`);
          buildOutput.show(true);
          window.showErrorMessage(`MCC build failed: ${msg}`);
          return;
        }
        renderBuildResult(filePath, result);
      } catch (e) {
        buildOutput.appendLine(`[mcc build] RPC error: ${String(e)}`);
        buildOutput.show(true);
        window.showErrorMessage(`MCC build error: ${String(e)}`);
      }
    }
  );
}

// Write the build report to the "MCode Build" output channel and map the
// build's warnings/errors into the Problems tab.
function renderBuildResult(entryFile: string, result: BuildResult): void {
  const s = result.summary ?? {};
  const diags = result.diagnostics ?? [];

  // ── Output console ──
  buildOutput.appendLine("");
  buildOutput.appendLine(`[mcc build] entry: ${entryFile}`);
  buildOutput.appendLine(
    `[mcc build] modules: ${s.module_count ?? 0}, components: ${
      s.component_count ?? 0
    }, interfaces: ${s.interface_count ?? 0}, instances: ${
      s.instance_count ?? 0
    }, nets: ${s.net_count ?? 0}`
  );

  // ── Categorized statistics (mirrors `mcc build`'s Summary block) ──
  // namespace classes / used classes split into system (`/mcode/`) vs project
  // space, and the instance breakdown by kind.
  const st = s.stats;
  if (st) {
    const z = (n?: number) => n ?? 0;
    buildOutput.appendLine(
      `[mcc build]   namespace classes: modules=${
        z(st.ns_modules_system) + z(st.ns_modules_project)
      }, components=${
        z(st.ns_components_system) + z(st.ns_components_project)
      }, interfaces=${
        z(st.ns_interfaces_system) + z(st.ns_interfaces_project)
      }`
    );
    buildOutput.appendLine(
      `[mcc build]     system:  modules=${z(st.ns_modules_system)}, components=${z(
        st.ns_components_system
      )}, interfaces=${z(st.ns_interfaces_system)}`
    );
    buildOutput.appendLine(
      `[mcc build]     project: modules=${z(st.ns_modules_project)}, components=${z(
        st.ns_components_project
      )}, interfaces=${z(st.ns_interfaces_project)}`
    );
    buildOutput.appendLine(
      `[mcc build]   used classes:      modules=${
        z(st.used_modules_system) + z(st.used_modules_project)
      }, components=${
        z(st.used_components_system) + z(st.used_components_project)
      }`
    );
    buildOutput.appendLine(
      `[mcc build]     system:  modules=${z(st.used_modules_system)}, components=${z(
        st.used_components_system
      )}`
    );
    buildOutput.appendLine(
      `[mcc build]     project: modules=${z(st.used_modules_project)}, components=${z(
        st.used_components_project
      )}`
    );
    buildOutput.appendLine(
      `[mcc build]   instances:         ${
        s.instance_count ?? 0
      } (modules=${z(st.module_insts)}, components=${z(st.component_insts)})`
    );
  }

  buildOutput.appendLine(
    `[mcc build] errors: ${s.errors ?? 0}, warnings: ${
      s.warnings ?? 0
    } (${s.elapsed_ms ?? 0} ms)`
  );
  for (const d of diags) {
    buildOutput.appendLine(
      `  ${d.severity} [${d.code}] ${d.file}:${d.line}:${d.column}: ${d.message}`
    );
  }
  buildOutput.show(true);

  // ── Problems tab (own collection, never clobbers live editing diagnostics) ──
  // Open files already get their diagnostics from the language server's
  // per-file collection. Skip build entries that duplicate a live one (same
  // code at the same position), so a warning like E5501 doesn't appear twice.
  const liveKeys = new Map<string, Set<string>>();
  const liveKey = (code: unknown, line: number, col: number) => `${code}|${line}|${col}`;
  const isLive = (uri: Uri, code: unknown, line: number, col: number): boolean => {
    let set = liveKeys.get(uri.toString());
    if (!set) {
      set = new Set();
      for (const d of languages.getDiagnostics(uri)) {
        set.add(liveKey(d.code, d.range.start.line, d.range.start.character));
      }
      liveKeys.set(uri.toString(), set);
    }
    return set.has(liveKey(code, line, col));
  };

  const byFile = new Map<string, Diagnostic[]>();
  for (const d of diags) {
    const uri = d.file.startsWith("file://") ? Uri.parse(d.file) : Uri.file(d.file);
    const line = Math.max(0, (d.line || 1) - 1);
    const column = Math.max(0, (d.column || 1) - 1);
    if (isLive(uri, d.code, line, column)) {
      continue; // already reported by the live per-file diagnostics
    }
    const len = Math.max(1, d.len || 1);
    const range = new Range(new Position(line, column), new Position(line, column + len));
    const diag = new Diagnostic(range, d.message, severityFor(d.severity));
    diag.source = "mcc build";
    diag.code = d.code;
    const arr = byFile.get(uri.toString()) ?? [];
    arr.push(diag);
    byFile.set(uri.toString(), arr);
  }
  buildDiags.clear();
  for (const [key, ds] of byFile) {
    buildDiags.set(Uri.parse(key), ds);
  }
}

function severityFor(s: string): DiagnosticSeverity {
  switch (s) {
    case "warning":
      return DiagnosticSeverity.Warning;
    case "info":
      return DiagnosticSeverity.Information;
    case "hint":
      return DiagnosticSeverity.Hint;
    default:
      return DiagnosticSeverity.Error;
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
