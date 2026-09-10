/* --------------------------------------------------------------------------------------------
 * Copyright (c) Microsoft Corporation. All rights reserved.
 * Licensed under the MIT License. See License.txt in the project root for license information.
 * ------------------------------------------------------------------------------------------ */

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
  TabInputCustom,
  WebviewPanel,
  CustomDocument,
  CustomReadonlyEditorProvider,
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

// viz circuit preview is a *per-file custom editor* (`mcode.viz`, see the
// customEditors contribution in package.json): opening it creates a normal
// editor tab — like a code file — that can be moved to another group or dragged
// into its own window, instead of a fixed side-by-side pane.
const VIZ_VIEW_TYPE = "mcode.viz";

// Optional top-module override for no-project files, passed by the previewViz
// command; keyed by the .mc file's path so re-opening that file's preview
// (or re-rendering after edits) honours it. Empty for the common case where
// mcc picks the file's first module itself.
const vizTopByPath = new Map<string, string>();

// Whole-project build state: the Output console the `mcc build` report is
// written to, and a dedicated DiagnosticCollection so build warnings/errors
// appear in the Problems tab WITHOUT clobbering the live per-file diagnostics
// published by the language server during editing.
const buildOutput = window.createOutputChannel("MCode Build");
const buildDiags = languages.createDiagnosticCollection("mcode-build");

// Live-vs-build dedup identity: a language-server diagnostic (source "mcc") and
// a whole-project build diagnostic (source "mcc build") that share the same code
// and 0-based start position describe the same problem — showing both in the
// Problems tab is duplication.
function diagnosticKey(code: unknown, line: number, col: number): string {
  return `${code}|${line}|${col}`;
}

// True when the language server has already published a per-file diagnostic
// matching the build entry at (code, line, col). The live collection is the
// language client's own `client.diagnostics` (created lazily on the first
// publishDiagnostics), kept separate from buildDiags so a build never clobbers
// editing-time diagnostics.
function hasLiveDiag(uri: Uri, code: unknown, line: number, col: number): boolean {
  const target = diagnosticKey(code, line, col);
  for (const d of client.diagnostics?.get(uri) ?? []) {
    if (diagnosticKey(d.code, d.range.start.line, d.range.start.character) === target) {
      return true;
    }
  }
  return false;
}

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
  // Failure ledger (resolve-gate-design.md §7.1-2): cross-pass record of
  // non-clean parses — silent fallbacks, phantoms, floating wires.
  ledger?: {
    total: number;
    by_kind_form: { [kind: string]: { [form: string]: number } };
    resolved_late: number;
    detail?: {
      kind: string;
      form: string;
      site: string;
      action: string;
      refs?: number;
      file?: string;
      line?: number;
      column?: number;
      pos?: number;
      len?: number;
    }[];
  };
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

  // viz circuit preview: open the active .mc file's circuit as its own editor
  // tab (custom editor `mcode.viz`) instead of a fixed side-by-side pane.
  // Optional command argument = top module name for no-project files; when
  // omitted, mcc falls back to the first module of the file (usually `main`).
  context.subscriptions.push(
    commands.registerCommand("mcode.previewViz", (top?: string) => previewViz(top))
  );

  // The per-file circuit custom editor provider. retainContextWhenHidden keeps
  // the JS-rendered schematic alive across tab switches, so the preview doesn't
  // blank and re-render every time you go to a code tab and back.
  const previewProvider = new CircuitPreviewProvider(client, clientStarted);
  context.subscriptions.push(
    window.registerCustomEditorProvider(VIZ_VIEW_TYPE, previewProvider, {
      webviewOptions: { retainContextWhenHidden: true },
    }),
    previewProvider
  );

  // Auto-show the active .mc's circuit once, when VSCode opens with an .mc file
  // active — or the first .mc document becomes active — preserving the earlier
  // auto-preview behaviour. One-shot per activation: after it fires, the
  // preview is an ordinary editor tab the user opens/closes like any file, so a
  // tab closed deliberately stays closed until the command runs again.
  let autoPreviewFired = false;
  const autoOpenPreview = (): void => {
    if (autoPreviewFired) return;
    const editor = window.activeTextEditor;
    if (!editor || editor.document.languageId !== "mcode") return;
    autoPreviewFired = true;
    void previewViz();
  };
  autoOpenPreview();
  context.subscriptions.push(window.onDidChangeActiveTextEditor(autoOpenPreview));

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

  // Reconcile build diagnostics against live ones after the fact. The language
  // server publishes per-file diagnostics asynchronously (debounced reparse
  // after open/change/save, serialized behind mcc's single-threaded RPC lock),
  // so a build can finish before the live copy of a problem exists — the
  // pre-filter inside buildProject couldn't see it, and both a "mcc" and a
  // "mcc build" entry would end up in Problems. Whenever a document's live
  // diagnostics change, drop any build entry for it that now duplicates one.
  // Idempotent: if nothing is removed we don't touch the collection, so this
  // never re-triggers itself in a loop.
  context.subscriptions.push(
    languages.onDidChangeDiagnostics((e) => {
      for (const uri of e.uris) {
        const build = buildDiags.get(uri);
        if (!build || build.length === 0) continue;
        const live = client.diagnostics?.get(uri);
        if (!live || live.length === 0) continue;
        const liveKeys = new Set(
          live.map((d) => diagnosticKey(d.code, d.range.start.line, d.range.start.character))
        );
        const kept = build.filter(
          (d) => !liveKeys.has(diagnosticKey(d.code, d.range.start.line, d.range.start.character))
        );
        if (kept.length === build.length) continue; // nothing duplicated
        if (kept.length === 0) {
          buildDiags.delete(uri);
        } else {
          buildDiags.set(uri, kept);
        }
      }
    })
  );

}

// Open the active .mc file's circuit as its own editor tab — the `mcode.viz`
// custom editor — in the current editor group, like opening a code file. The
// user can then move that tab to another group or drag it out into its own
// window. Repeated invocation reuses the open preview instead of stacking
// duplicates (registerCustomEditorProvider keeps one editor per resource).
async function previewViz(top?: string): Promise<void> {
  const editor = window.activeTextEditor;
  if (!editor || editor.document.languageId !== "mcode") {
    window.showInformationMessage(
      "MCode: open a .mc file to preview its circuit."
    );
    return;
  }

  const uri = editor.document.uri;
  if (top) vizTopByPath.set(uri.fsPath, top);
  try {
    await commands.executeCommand("vscode.openWith", uri, VIZ_VIEW_TYPE);
  } catch (e) {
    window.showErrorMessage(`MCode: circuit preview failed to open: ${String(e)}`);
    return;
  }

  // openWith resolves even when no custom editor matches the resource (it just
  // re-focuses the plain text editor), so confirm a `mcode.viz` tab really
  // appeared before claiming success.
  if (!(await waitForCircuitTab(uri))) {
    window.showErrorMessage(
      `MCode: no circuit preview tab opened for ${path.basename(uri.fsPath)}. ` +
        "The package.json customEditors contribution may not be loaded — run " +
        "Developer: Reload Window and retry."
    );
  }
}

// True when a `mcode.viz` custom-editor tab for `uri` is currently open.
function hasCircuitTab(uri: Uri): boolean {
  const u = uri.toString();
  for (const group of window.tabGroups.all) {
    for (const tab of group.tabs) {
      const input = tab.input;
      if (
        input instanceof TabInputCustom &&
        input.viewType === VIZ_VIEW_TYPE &&
        input.uri.toString() === u
      ) {
        return true;
      }
    }
  }
  return false;
}

// Poll briefly for the custom-editor tab: openWith resolves before the editor
// settles in some VS Code versions, so wait a few ticks before giving up.
async function waitForCircuitTab(uri: Uri): Promise<boolean> {
  for (let i = 0; i < 10; i++) {
    if (hasCircuitTab(uri)) return true;
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  return hasCircuitTab(uri);
}

// Minimal readonly document for the circuit custom editor: it never edits the
// .mc — the webview only displays the schematic derived from it.
class CircuitDocument implements CustomDocument {
  constructor(public readonly uri: Uri) {}
  dispose(): void {}
}

// Per-file circuit preview: resolves each .mc's `mcode.viz` editor into a
// webview showing the schematic the language server renders for that file.
// The preview is an ordinary editor tab — switch to it like a code tab, drag it
// to another group or window — and live-refreshes (debounced) as the source
// edits, mirroring the language server's own debounced reparse.
class CircuitPreviewProvider implements CustomReadonlyEditorProvider<CircuitDocument> {
  // Open preview webviews by source-uri string; each entry carries a render
  // sequence so a slow RPC response can't overwrite a newer render or a
  // re-render scheduled after the panel was closed.
  private readonly panels = new Map<string, { panel: WebviewPanel; seq: number }>();
  // Pending debounced re-render timers per source uri.
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly changeSub: Disposable;

  constructor(
    private readonly client: LanguageClient,
    private readonly clientStarted: Promise<void> | undefined
  ) {
    this.changeSub = workspace.onDidChangeTextDocument((e) => {
      const key = e.document.uri.toString();
      if (e.document.languageId !== "mcode" || !this.panels.has(key)) return;
      const pending = this.timers.get(key);
      if (pending) clearTimeout(pending);
      this.timers.set(
        key,
        setTimeout(() => {
          this.timers.delete(key);
          void this.rerender(key, e.document.uri.fsPath);
        }, 400)
      );
    });
  }

  dispose(): void {
    this.changeSub.dispose();
    for (const t of this.timers.values()) clearTimeout(t);
    this.timers.clear();
  }

  openCustomDocument(uri: Uri): CircuitDocument {
    return new CircuitDocument(uri);
  }

  resolveCustomEditor(document: CircuitDocument, panel: WebviewPanel): void {
    // The returned schematic is a small self-contained JS app (mcc viz renders
    // its SVG into the DOM via <script>), so scripts must be enabled.
    panel.webview.options = { ...panel.webview.options, enableScripts: true };
    const key = document.uri.toString();
    const entry = { panel, seq: 0 };
    this.panels.set(key, entry);
    panel.onDidDispose(() => {
      if (this.panels.get(key) === entry) this.panels.delete(key);
      const pending = this.timers.get(key);
      if (pending) clearTimeout(pending);
      this.timers.delete(key);
    });
    void this.render(key, document.uri.fsPath, panel, entry.seq);
  }

  private async rerender(key: string, fsPath: string): Promise<void> {
    const entry = this.panels.get(key);
    if (!entry) return;
    entry.seq += 1;
    await this.render(key, fsPath, entry.panel, entry.seq);
  }

  // Ask the language server to build the circuit for `fsPath`, then load the
  // returned HTML. `seq` guards against stale responses and a panel disposed
  // mid-flight (dispose removes the map entry before our continuation runs).
  private async render(
    key: string,
    fsPath: string,
    panel: WebviewPanel,
    seq: number
  ): Promise<void> {
    panel.webview.html = loadingHtml("Rendering circuit…");
    try {
      if (this.clientStarted) {
        await this.clientStarted;
      }
      const cur = this.panels.get(key);
      if (!cur || cur.seq !== seq) return;
      const top = vizTopByPath.get(fsPath);
      const result = (await this.client.sendRequest("workspace/executeCommand", {
        command: "mcode.viz",
        arguments: top ? [fsPath, top] : [fsPath],
      })) as { ok: boolean; html?: string; error?: string } | null;
      const after = this.panels.get(key);
      if (!after || after.seq !== seq) return;
      panel.webview.html = previewResultHtml(result);
    } catch (e) {
      const after = this.panels.get(key);
      if (!after || after.seq !== seq) return;
      panel.webview.html = errorHtml(String(e));
    }
  }
}

// Run a whole-project build via the language server (daemon `build.full` RPC),
// then render the report to the Output console and warnings/errors to the
// Problems tab.
async function buildProject(): Promise<void> {
  // Unified principle (mcd use-design §19.5 rule 3): Build Project targets a
  // *file* when the active editor is an .mc file, otherwise a *folder*. The
  // server's build.full directory branch batch-parses every .mc file under the
  // folder (including subfolders) when it has no project.toml/manifest.toml/
  // mcc.toml, or builds the project when one is present.
  const editor = window.activeTextEditor;
  let entryPath: string | undefined;

  if (editor && editor.document.languageId === "mcode") {
    entryPath = editor.document.uri.fsPath;
  } else {
    // No active .mc editor → build the opened workspace folder (or, failing
    // that, the directory containing the active file).
    const folder = workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (folder) {
      entryPath = folder;
    } else if (editor) {
      entryPath = path.dirname(editor.document.uri.fsPath);
    }
  }

  if (!entryPath) {
    window.showInformationMessage(
      "MCode: open a .mc file or a folder to build."
    );
    return;
  }

  const filePath = entryPath;

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

  // ── Failure ledger (resolve-gate-design.md §7.1): silent non-clean parses ──
  const ledger = result.ledger;
  if (ledger) {
    buildOutput.appendLine(
      `[mcc build] ledger: ${ledger.total} non-clean ${
        ledger.total === 1 ? "parse" : "parses"
      } (deferred resolved late: ${ledger.resolved_late ?? 0})`
    );
    for (const [kind, forms] of Object.entries(ledger.by_kind_form)) {
      const entries = Object.entries(forms);
      if (entries.length === 0) continue;
      const count = entries.reduce((a, [, c]) => a + c, 0);
      const formsStr = entries.map(([f, c]) => `${f}×${c}`).join(", ");
      buildOutput.appendLine(`[mcc build]   ${kind}: ${count} [${formsStr}]`);
    }
  }

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
  // This pre-filter only sees live diagnostics published so far — live ones that
  // land *after* the build completes are handled by the onDidChangeDiagnostics
  // reconciler registered in activate() (which prunes duplicate build entries
  // whenever the language server re-publishes a document's diagnostics).
  const byFile = new Map<string, Diagnostic[]>();
  for (const d of diags) {
    const uri = d.file.startsWith("file://") ? Uri.parse(d.file) : Uri.file(d.file);
    const line = Math.max(0, (d.line || 1) - 1);
    const column = Math.max(0, (d.column || 1) - 1);
    if (hasLiveDiag(uri, d.code, line, column)) {
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

// RPC error → webview content. A file that declares no module, component, or
// interface (e.g. a pure reference file like units.mc) is not an error worth
// showing: the server reports "no module, component, or interface found"
// (virtual_inst.rs resolve_targets), which just means there is nothing to
// render — show a neutral notice instead of the raw RPC error.
function previewResultHtml(
  result: { ok: boolean; html?: string; error?: string } | null
): string {
  if (result && result.ok && result.html) {
    return result.html;
  }
  const err = result?.error ?? "";
  if (/no module, component, or interface found/i.test(err)) {
    return noCircuitHtml();
  }
  return errorHtml(err || "build.viz returned no html");
}

function noCircuitHtml(): string {
  return `<!DOCTYPE html><html><body><p>No circuit to display — this file declares no module, component, or interface.</p></body></html>`;
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
