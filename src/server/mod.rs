//! LSP Server: Backend struct + LanguageServer implementation
//!
//! Phase 0 keeps existing LSP behavior (goto-definition, references, semantic tokens, diagnostics),
//! all handlers internally delegate to `features::*` module. Zero behavior changes.
//!
//! Phase 2 incremental sync + debounce:
//! - TextDocumentSyncKind::INCREMENTAL
//! - ReparseScheduler debounce 150ms
//! - version validation to prevent out-of-order diagnostics

use crate::common::ServerConfig;
use crate::index::IndexCommand;
use crate::mccsrv::MccServer;
use crate::project::ProjectConfig;
use crate::state::WorkspaceState;
use dashmap::DashMap;
// Note: McURI is just String, no need to import mcc
use ropey::Rope;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{debug, info, trace, warn};

/// mcode LSP server
pub struct Backend {
    pub client: Client,
    pub state: Arc<WorkspaceState>,
    /// Config (updated by did_change_configuration after startup)
    pub config: Arc<DashMap<String, ServerConfig>>,
    /// MCC server subprocess manager (for RPC mode)
    pub mcc_server: Arc<tokio::sync::RwLock<Option<MccServer>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: Arc::new(WorkspaceState::with_worker()),
            config: Arc::new(DashMap::new()),
            // Create mcc_server with default config, will be started in initialize()
            mcc_server: Arc::new(tokio::sync::RwLock::new(Some(MccServer::new()))),
        }
    }

    /// Synchronous path for did_save: immediate parse + publish
    async fn on_change_full(&self, uri: Url, text: &str, version: Option<i32>) {
        debug!("on_change_full: {}", uri.path());
        let rope = Rope::from_str(text);
        let ver = version.unwrap_or(-1);
        self.state.insert_document(uri.clone(), rope.clone(), ver);

        // Fire immediately, bypass debounce
        let state = Arc::clone(&self.state);
        let mcc_server = Arc::clone(&self.mcc_server);
        let client = self.client.clone();
        let uri_for_task = uri.clone();
        self.state.project.scheduler.fire_immediately(uri, move || {
            tokio::spawn(parse_and_publish(
                state,
                mcc_server,
                client,
                uri_for_task,
                version,
            ));
        });
    }

    /// Incremental path for did_change: apply changes + schedule debounced reparse
    async fn on_change_incremental(
        &self,
        uri: Url,
        changes: &[TextDocumentContentChangeEvent],
        version: Option<i32>,
    ) {
        debug!("on_change_incremental: {}", uri.path());

        // Get Rope, apply changes
        let mut rope = self
            .state
            .document_rope(&uri)
            .unwrap_or_else(|| Rope::from_str(""));
        if let Err(e) = crate::state::apply_changes(&mut rope, changes) {
            debug!("apply_changes failed: {e}");
            return;
        }

        // ★ Adjust cached lapper offsets so F12 is accurate before the
        // debounced reparse completes.
        crate::state::adjust_lapper_for_changes(&self.state, &uri, changes, &rope);

        let ver = version.unwrap_or(-1);
        self.state.insert_document(uri.clone(), rope, ver);

        // Schedule debounced reparse
        let state = Arc::clone(&self.state);
        let mcc_server = Arc::clone(&self.mcc_server);
        let client = self.client.clone();
        let uri_for_task = uri.clone();
        self.state.project.scheduler.schedule(uri, move || {
            tokio::spawn(parse_and_publish(
                state,
                mcc_server,
                client,
                uri_for_task,
                version,
            ));
        });
    }

    /// Build the ServerCapabilities response (extracted from initialize).
    fn build_capabilities() -> ServerCapabilities {
        ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::INCREMENTAL),
                    save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                        include_text: Some(true),
                    })),
                    ..Default::default()
                },
            )),
            workspace: Some(WorkspaceServerCapabilities {
                workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                    supported: Some(true),
                    change_notifications: Some(OneOf::Left(true)),
                }),
                file_operations: None,
            }),
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
                    SemanticTokensRegistrationOptions {
                        text_document_registration_options: TextDocumentRegistrationOptions {
                            document_selector: Some(vec![DocumentFilter {
                                language: Some("mcode".to_string()),
                                scheme: Some("file".to_string()),
                                pattern: None,
                            }]),
                        },
                        semantic_tokens_options: SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                            legend: SemanticTokensLegend {
                                token_types: crate::common::LEGEND_TYPE.into(),
                                token_modifiers: vec![],
                            },
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                        },
                        static_registration_options: StaticRegistrationOptions::default(),
                    },
                ),
            ),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            completion_provider: Some(CompletionOptions {
                resolve_provider: Some(true),
                trigger_characters: Some(vec![".".to_string(), ":".to_string(), " ".to_string()]),
                all_commit_characters: Some(vec![]),
                completion_item: None,
                work_done_progress_options: WorkDoneProgressOptions::default(),
            }),
            document_formatting_provider: Some(OneOf::Left(true)),
            document_range_formatting_provider: Some(OneOf::Left(true)),
            inlay_hint_provider: Some(OneOf::Left(true)),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            ..ServerCapabilities::default()
        }
    }
}

/// MCC server initialization task: start subprocess, load project, fetch symbols,
/// retry pending diagnostics. Runs in a background tokio task during initialize.
async fn run_server_init(
    mcc_server: Arc<tokio::sync::RwLock<Option<MccServer>>>,
    project_root: Option<std::path::PathBuf>,
    system_root: Option<std::path::PathBuf>,
    state: Arc<WorkspaceState>,
    lsp_client: Client,
) {
    // Phase 1: start mcc server, clone RPC client, release write lock
    let rpc_client: Option<crate::rpc::MccRpcClient> = {
        let mut server_guard = mcc_server.write().await;
        let Some(ref mut server) = *server_guard else {
            return;
        };
        if let Err(e) = server.start().await {
            warn!(
                "Failed to start mcc server, falling back to direct mode: {}",
                e
            );
            return;
        }
        debug!("mcc server subprocess started");
        server.client().cloned()
    };

    let Some(client) = rpc_client else {
        warn!("mcc server started but RPC client is None");
        return;
    };

    // Phase 2: initialize project (no lock held — parse_and_publish runs concurrently)
    if let Some(ref sys_root) = system_root {
        info!("Setting system root: {}", sys_root.display());
        let _ = client
            .set_system_root(sys_root.to_str().unwrap_or(""))
            .await;
    }

    let _ = client.init().await;

    if let Some(ref root) = project_root {
        let root_str = root.to_string_lossy();
        let _ = client.set_project_root(&root_str).await;

        if let Some(config) = ProjectConfig::load_from(root) {
            info!(
                "Auto-loading {} dependencies from project.toml...",
                config.dependency_names().len()
            );
            for lib_name in config.dependency_names() {
                info!("Calling lib.load for: {}", lib_name);
                let lib_result = client.lib_load(lib_name).await;
                if lib_result.is_ok() {
                    info!("Successfully loaded lib: {}", lib_name);
                } else {
                    warn!("Failed to load lib '{}': {:?}", lib_name, lib_result.err());
                }
                match client.lib_show(lib_name).await {
                    Ok(info) => {
                        info!(
                            "Lib info: name={} symbols={} modules={} components={} interfaces={}",
                            info.name,
                            info.total_symbols,
                            info.module_count,
                            info.component_count,
                            info.interface_count
                        );
                    }
                    Err(e) => {
                        warn!("lib_show failed: {:?}", e);
                    }
                }
            }

            let entry = config.entry_path(root);
            info!("Calling load_project for entry: {}", entry);
            if let Err(e) = client.load_project(&entry).await {
                warn!("Failed to load project '{}': {:?}", entry, e);
            } else {
                info!("Successfully loaded project entry: {}", entry);
            }

            // Fetch project symbols for the index (incl. enum values)
            info!("Fetching project_symbols for index...");
            if let Ok(resp) = client.project_symbols().await {
                let ec = resp.enums.clone();
                let ev = resp.enum_values.clone();
                if let Ok(mut cache) = state.symbols.project_symbols.lock() {
                    cache.components = resp.components;
                    cache.interfaces = resp.interfaces;
                    cache.enums = ec.clone();
                    cache.modules = resp.modules;
                    cache.enum_values = ev.clone();
                }
                if let Ok(cache) = state.symbols.project_symbols.lock() {
                    let _ = state.project.index.send(
                        crate::index::worker::IndexCommand::UpdateProjectSymbols {
                            components: cache.components.clone(),
                            interfaces: cache.interfaces.clone(),
                            enums: cache.enums.clone(),
                            modules: cache.modules.clone(),
                            enum_values: cache.enum_values.clone(),
                        },
                    );
                }
                info!("project_symbols done → worker updated");
            } else {
                warn!("project_symbols RPC failed");
            }
        }
    }

    // Signal init complete (order: sticky flag first, then notify)
    state.init.done.store(true, Ordering::Release);
    state.init.notify.notify_waiters();
    info!("init_done = true — parse_and_publish can now make RPC calls");

    // Phase 3: retry pending diagnostics
    let pending: Vec<(Url, Option<i32>)> = state
        .diags
        .pending
        .iter()
        .map(|entry| (entry.key().clone(), *entry.value()))
        .collect();
    for (uri, _) in &pending {
        state.diags.pending.remove(uri);
    }
    if !pending.is_empty() {
        info!(
            "Retrying {} pending diagnostics after mcc server ready",
            pending.len()
        );
        for (uri, version) in pending {
            if state.docs.documents.contains_key(&uri) {
                let s = Arc::clone(&state);
                let mc = Arc::clone(&mcc_server);
                let cl = lsp_client.clone();
                parse_and_publish(s, mc, cl, uri, version).await;
            }
        }
    }
}

/// Parse + publish diagnostics (executed in debounced task)
async fn parse_and_publish(
    state: Arc<WorkspaceState>,
    mcc_server: Arc<tokio::sync::RwLock<Option<MccServer>>>,
    client: Client,
    uri: Url,
    version: Option<i32>,
) {
    let span = tracing::info_span!("parse_and_publish", uri = %uri.path(), ?version);
    let _guard = span.enter();

    debug!("parse_and_publish ENTER: uri={}", uri.path());
    let mc_uri = String::from(uri.path());

    // Guard against mcc SIGABRT/SIGSEGV: validate use paths first (warn only, non-blocking)
    let text = state
        .document_rope(&uri)
        .map(|r| r.to_string())
        .unwrap_or_default();
    if let crate::util::UseCheckResult::Missing {
        use_line,
        candidates,
    } = crate::util::check_use_targets(&uri, &text)
    {
        warn!("use target missing for {uri}: {use_line} (tried: {candidates:?})");
    }

    // Wait for project initialization to complete before making RPC calls.
    // Concurrent RPC calls during init can crash the single-threaded mcc server.
    //
    // Two-phase: AtomicBool for sticky check (fast path after init),
    //            Notify for efficient wakeup (slow path during init).
    if !state.init.done.load(Ordering::Acquire) {
        // Slow path: wait for init_notify.  After waking, MUST re-check
        // init_done because Notify is NOT sticky — if notify_waiters()
        // was already consumed by earlier tasks, notified() would block
        // forever.  The AtomicBool protects against this TOCTOU race.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            state.init.notify.notified(),
        )
        .await;
        if !state.init.done.load(Ordering::Acquire) {
            // Still not done after wake/timeout → truly not ready
            info!("parse_and_publish: init not ready for {uri}, queuing for retry");
            state.diags.pending.insert(uri.clone(), version);
            return;
        }
    }

    let server_guard = mcc_server.read().await;
    let Some(server) = server_guard.as_ref() else {
        debug!("mcc server not available for {uri}, queuing for retry");
        state.diags.pending.insert(uri.clone(), version);
        return;
    };

    if !server.is_connected() {
        // Wait briefly for connection (server may have crashed and be restarting)
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if server.is_connected() {
                break;
            }
        }
        if !server.is_connected() {
            debug!("mcc server still not connected for {uri}, queuing for retry");
            state.diags.pending.insert(uri.clone(), version);
            return;
        }
    }

    let uri_str = uri.path();
    // Serialize RPC access: mcc is single-threaded, concurrent requests crash it.
    let _rpc_guard = state.rpc_lock.lock().await;
    // Pass current document text so mcc parses live content (not stale disk file)
    let content_for_rpc: Option<&str> = Some(&text);
    let sem = match server.sem(uri_str, content_for_rpc).await {
        Ok(sem) => sem,
        Err(e) => {
            debug!("sem RPC FAILED for {uri}: {e}");
            return;
        }
    };

    // Get diagnostics via RPC
    let mut diagnostics = Vec::new();
    match server.diagnostics(uri_str).await {
        Ok(resp) => {
            debug!(
                "diagnostics RPC OK for {}: {} diags",
                uri_str,
                resp.diagnostics.len()
            );
            let Some(rope) = state.document_rope(&uri) else {
                tracing::warn!("document rope missing for {} — skipping diagnostics", uri);
                return;
            };
            for d in &resp.diagnostics {
                debug!(
                    "  diag: code={} line={} col={} pos={} len={} msg={}",
                    d.code,
                    d.location.line,
                    d.location.column,
                    d.location.pos,
                    d.location.len,
                    d.message
                );
            }
            for d in resp.diagnostics {
                // DEBUG: log raw diagnostic data
                // debug!(
                //     "diag: code={} line={} col={} pos={} len={} msg={}",
                //     d.code, d.location.line, d.location.column, d.location.pos, d.location.len, d.message
                // );

                // Prefer RPC-provided line/column (from mcc's Location::new which computes them correctly)
                // Fall back to pos-based conversion if line=0 OR if line=1,col=1 but pos indicates
                // a later position (pos_to_line_col failed silently, returned default)
                let rpc_pos_ok = d.location.line > 1
                    || d.location.column > 1
                    || (d.location.line == 1 && d.location.column == 1 && d.location.pos == 0);
                let start = if rpc_pos_ok && d.location.line > 0 {
                    // RPC provides 1-based line/column, LSP expects 0-based
                    tower_lsp::lsp_types::Position::new(
                        d.location.line - 1,
                        d.location.column.saturating_sub(1),
                    )
                } else {
                    // Fallback: convert pos to position using rope
                    match crate::common::position::offset_to_position(
                        d.location.pos as usize,
                        &rope,
                    ) {
                        Some(s) => s,
                        None => continue,
                    }
                };

                // When using RPC-provided line/column, calculate end from pos+len but clamp to same line
                // (the len is based on AST node size which may span multiple lines)
                // When falling back to pos-based conversion, calculate end from pos+len
                let end = match crate::common::position::offset_to_position(
                    (d.location.pos + d.location.len) as usize,
                    &rope,
                ) {
                    Some(e) => {
                        // Clamp end to same line as start to avoid multi-line spans
                        if e.line > start.line {
                            crate::common::position::line_end_position(start.line, &rope)
                        } else {
                            e
                        }
                    }
                    None => {
                        // If we can't calculate end, use line end
                        crate::common::position::line_end_position(start.line, &rope)
                    }
                };

                // debug!("  -> start=({}, {}) end=({}, {})", start.line, start.character, end.line, end.character);

                let severity = match d.level.as_str() {
                    "error" => tower_lsp::lsp_types::DiagnosticSeverity::ERROR,
                    "warning" => tower_lsp::lsp_types::DiagnosticSeverity::WARNING,
                    "info" => tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION,
                    "hint" => tower_lsp::lsp_types::DiagnosticSeverity::HINT,
                    _ => tower_lsp::lsp_types::DiagnosticSeverity::ERROR,
                };
                diagnostics.push(tower_lsp::lsp_types::Diagnostic::new(
                    tower_lsp::lsp_types::Range::new(start, end),
                    Some(severity),
                    Some(tower_lsp::lsp_types::NumberOrString::Number(d.code as i32)),
                    Some("mcc".into()),
                    d.message,
                    None,
                    None,
                ));
            }
        }
        Err(e) => {
            debug!("diagnostics RPC FAILED for {uri}: {e}");
        }
    }

    drop(server_guard);

    let rpc_tokens = crate::state::RpcSemTokens {
        tokens: sem
            .tokens
            .into_iter()
            .map(|t| crate::state::SemTokenEntry {
                type_: t.token_type,
                position: t.position,
                length: t.length,
            })
            .collect(),
    };
    state
        .symbols
        .sem_tokens
        .insert(uri.clone(), Arc::new(std::sync::Mutex::new(rpc_tokens)));
    state
        .docs
        .registered_uris
        .insert(uri.clone(), mc_uri.clone());

    // ★ Fix: Store sem_symbols from RPC response for goto_definition and other features
    let rpc_symbols = crate::state::RpcSemSymbols::from(sem.symbols);
    state
        .symbols
        .sem_symbols
        .insert(uri.clone(), Arc::new(std::sync::Mutex::new(rpc_symbols)));

    // Store the result_id so semantic_tokens_full uses it
    if let Some(rid) = sem.result_id {
        let lsp_tokens = crate::features::semtok::compute(&state, &uri).unwrap_or_default();
        state
            .symbols
            .tokens
            .store_with_result_id(uri.clone(), rid, lsp_tokens);
    }

    // Trigger semantic tokens refresh so VSCode re-requests after parse
    client.semantic_tokens_refresh().await.ok();

    // Always publish diagnostics (with current version) so old errors get cleared.
    // The diagnostics vector contains results from the latest mcc parse for this document.
    let diag_count = diagnostics.len();
    debug!("publish_diagnostics: {diag_count} diags for {}", uri.path());
    client
        .publish_diagnostics(uri.clone(), diagnostics, state.document_version(&uri))
        .await;

    // Success — remove from pending retry set if it was there
    state.diags.pending.remove(&uri);
    info!(
        "parse_and_publish done for {}: {diag_count} diags",
        uri.path()
    );
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let cfg = params
            .initialization_options
            .map(ServerConfig::from_initialization_options)
            .unwrap_or_default();

        let project_root = cfg
            .project_root
            .clone()
            .or_else(|| {
                params
                    .workspace_folders
                    .as_ref()
                    .and_then(|f| f.first())
                    .and_then(|f| f.uri.to_file_path().ok())
            })
            .or_else(|| std::env::current_dir().ok());

        // Background task: start mcc subprocess, load project + dependencies,
        // fetch symbols, build index, signal init-done, retry pending diagnostics.
        tokio::spawn(run_server_init(
            self.mcc_server.clone(),
            project_root.clone(),
            cfg.system_root.clone(),
            self.state.clone(),
            self.client.clone(),
        ));

        // Parse any documents VS Code already reported as open.
        // (initialized fires before didOpen, so this list is usually empty.)
        let docs: Vec<_> = self
            .state
            .docs
            .documents
            .iter()
            .map(|e| (e.key().clone(), e.value().version))
            .collect();
        for (uri, version) in docs {
            let s = Arc::clone(&self.state);
            let mc = Arc::clone(&self.mcc_server);
            let cl = self.client.clone();
            let u = uri.clone();
            let v = version;
            self.state.project.scheduler.fire_immediately(uri, move || {
                tokio::spawn(parse_and_publish(s, mc, cl, u, Some(v)));
            });
        }

        // Trigger project index scan
        if let Some(root) = project_root {
            let _ = self
                .state
                .project
                .index
                .send(IndexCommand::ParseAll(root.clone()));
            trace!(
                "server: project index ParseAll triggered: {}",
                root.display()
            );
        }

        self.config.insert("current".to_string(), cfg);

        Ok(InitializeResult {
            server_info: None,
            offset_encoding: None,
            capabilities: Self::build_capabilities(),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        debug!("initialized!");
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        debug!("did_open: {}", uri.path());

        // Store document and schedule parse immediately
        self.on_change_full(
            uri.clone(),
            &params.text_document.text,
            Some(params.text_document.version),
        )
        .await;

        // Notify index worker
        let mc_uri = String::from(uri.path());
        let _ = self.state.project.index.send(IndexCommand::AddFile(mc_uri));
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        debug!("did_change: {}", params.text_document.uri.path());
        let uri = params.text_document.uri.clone();
        // Phase 2: INCREMENTAL processing
        self.on_change_incremental(
            uri.clone(),
            &params.content_changes,
            Some(params.text_document.version),
        )
        .await;
        // Notify index worker
        let mc_uri = String::from(uri.path());
        let _ = self.state.project.index.send(IndexCommand::AddFile(mc_uri));
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        debug!("did_save: {}", params.text_document.uri.path());
        if let Some(text) = params.text {
            self.on_change_full(params.text_document.uri, &text, None)
                .await;
            let _ = self.client.semantic_tokens_refresh().await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        debug!("did_close: {}", params.text_document.uri.path());
        self.state.remove_document(&params.text_document.uri);
        self.state
            .project
            .scheduler
            .remove(&params.text_document.uri);
        let mc_uri = String::from(params.text_document.uri.path());
        let _ = self
            .state
            .project
            .index
            .send(IndexCommand::RemoveFile(mc_uri));
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        debug!("did_change_watched_files: {} changes", params.changes.len());
        for change in &params.changes {
            let mc_uri = String::from(change.uri.path());
            match change.typ {
                FileChangeType::DELETED => {
                    let _ = self
                        .state
                        .project
                        .index
                        .send(IndexCommand::RemoveFile(mc_uri));
                }
                FileChangeType::CREATED | FileChangeType::CHANGED => {
                    let _ = self.state.project.index.send(IndexCommand::AddFile(mc_uri));
                }
                _ => {}
            }
        }
    }

    // Phase 4: auto-completion
    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let span = tracing::debug_span!("completion", uri = %params.text_document_position.text_document.uri.path());
        let _guard = span.enter();
        Ok(crate::features::comp::resolve(
            &self.state,
            &params.text_document_position,
        ))
    }

    // Phase 4: completionItem/resolve additional info
    async fn completion_resolve(&self, params: CompletionItem) -> Result<CompletionItem> {
        Ok(crate::features::comp::resolve_item(params))
    }

    // Phase 4.2: full document formatting
    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.clone();
        let span = tracing::debug_span!("formatting", uri = %uri.path());
        let _guard = span.enter();

        let rope = match self.state.document_rope(&uri) {
            Some(r) => r,
            None => return Ok(None),
        };

        let options = crate::features::fmt::FormatOptions::new();
        Ok(crate::features::fmt::format_document(
            &uri,
            &rope,
            Some(options),
        ))
    }

    // Phase 4.2: range formatting
    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.clone();
        let span = tracing::debug_span!("range_formatting", uri = %uri.path());
        let _guard = span.enter();

        let rope = match self.state.document_rope(&uri) {
            Some(r) => r,
            None => return Ok(None),
        };

        let options = crate::features::fmt::FormatOptions::new();
        Ok(crate::features::fmt::format_range(
            &uri,
            &rope,
            params.range,
            Some(options),
        ))
    }

    // Phase 4.3: inline hints
    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri.clone();
        let span = tracing::debug_span!("inlay_hint", uri = %uri.path());
        let _guard = span.enter();

        let _rope = match self.state.document_rope(&uri) {
            Some(r) => r,
            None => return Ok(None),
        };

        Ok(crate::features::inhint::compute(
            &self.state,
            &uri,
            params.range,
        ))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let pos = params.text_document_position_params.position;
        let span = tracing::debug_span!("goto_definition", uri = %uri.path(), line = pos.line, col = pos.character);
        let _guard = span.enter();

        // If sem_symbols cache is missing (e.g. file was opened before mcc was
        // ready), do an on-the-fly sem call to populate it.
        // Only do this AFTER init is complete — concurrent RPC during init
        // (e.g. with Phase 2's load_project) will crash the single-threaded mcc.
        if self.state.symbols.sem_symbols.get(&uri).is_none()
            && self
                .state
                .init
                .done
                .load(std::sync::atomic::Ordering::Acquire)
        {
            if let Some(rope) = self.state.document_rope(&uri) {
                let server_guard = self.mcc_server.read().await;
                if let Some(server) = server_guard.as_ref() {
                    if server.is_connected() {
                        let text: String = rope.to_string();
                        // Serialize with other RPC calls (init, parse_and_publish)
                        let _rpc_guard = self.state.rpc_lock.lock().await;
                        if let Ok(sem) = server.sem(uri.path(), Some(&text)).await {
                            let rpc_symbols = crate::state::RpcSemSymbols::from(sem.symbols);
                            info!(
                                "goto_definition: on-the-fly sem populated for {}",
                                uri.path()
                            );
                            self.state
                                .symbols
                                .sem_symbols
                                .insert(uri.clone(), Arc::new(std::sync::Mutex::new(rpc_symbols)));
                        }
                    }
                }
            }
        }

        Ok(crate::features::gotodef::resolve(&self.state, &uri, pos))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let pos = params.text_document_position.position;
        let include_decl = params.context.include_declaration;
        let span = tracing::debug_span!("references", uri = %uri.path(), line = pos.line, col = pos.character, include_decl);
        let _guard = span.enter();
        Ok(crate::features::refs::resolve(
            &self.state,
            &uri,
            pos,
            include_decl,
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let span =
            tracing::debug_span!("semantic_tokens_full", uri = %params.text_document.uri.path());
        let _guard = span.enter();
        let uri = params.text_document.uri;

        let tokens = crate::features::semtok::compute(&self.state, &uri);
        let tokens = tokens.unwrap_or_default();
        let result_id = self.state.symbols.tokens.next_id();
        self.state
            .symbols
            .tokens
            .store(uri.clone(), result_id, tokens.clone());

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: Some(result_id.to_string()),
            data: tokens,
        })))
    }

    // Phase 3: semantic_tokens_full_delta handler
    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>> {
        let span = tracing::debug_span!("semantic_tokens_full_delta", uri = %params.text_document.uri.path(), prev_id = %params.previous_result_id);
        let _guard = span.enter();
        let uri = params.text_document.uri;

        let curr = crate::features::semtok::compute(&self.state, &uri);
        let curr = curr.unwrap_or_default();

        // Try incremental: check last tokens
        let prev_tokens = self.state.symbols.tokens.get(&uri);
        let prev_id_str: String = params.previous_result_id;
        if let Some((stored_id, prev_tokens)) = prev_tokens {
            if stored_id == prev_id_str {
                // id matches, try diff
                if let Some(delta) = crate::features::semtok::compute_delta(&prev_tokens, &curr) {
                    let new_id = prev_id_str.clone();
                    self.state.symbols.tokens.store_with_result_id(
                        uri.clone(),
                        new_id.clone(),
                        curr,
                    );
                    return Ok(Some(SemanticTokensFullDeltaResult::TokensDelta(
                        tower_lsp::lsp_types::SemanticTokensDelta {
                            result_id: Some(new_id),
                            edits: delta.edits,
                        },
                    )));
                }
            }

            // fallback to full
            let new_id = self.state.symbols.tokens.next_id().to_string();
            self.state.symbols.tokens.store_with_result_id(
                uri.clone(),
                new_id.clone(),
                curr.clone(),
            );
            return Ok(Some(SemanticTokensFullDeltaResult::Tokens(
                SemanticTokens {
                    result_id: Some(new_id),
                    data: curr,
                },
            )));
        }

        // No previous data, return full
        let new_id = self.state.symbols.tokens.next_id().to_string();
        self.state
            .symbols
            .tokens
            .store_with_result_id(uri.clone(), new_id.clone(), curr.clone());
        Ok(Some(SemanticTokensFullDeltaResult::Tokens(
            SemanticTokens {
                result_id: Some(new_id),
                data: curr,
            },
        )))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        debug!("semantic_tokens_range: {}", params.text_document.uri.path());
        let uri = params.text_document.uri;
        let tokens = crate::features::semtok::compute(&self.state, &uri);
        let tokens = tokens.unwrap_or_default();
        Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens,
        })))
    }

    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {
        debug!("did_change_configuration");
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        debug!("did_change_workspace_folders: {:?}", params.event);
        // Project root changed -> re-index and load dependencies
        if let Some(root) = params
            .event
            .added
            .first()
            .and_then(|f| f.uri.to_file_path().ok())
        {
            let _ = self
                .state
                .project
                .index
                .send(IndexCommand::ParseAll(root.clone()));

            // Auto-load project dependencies from project.toml
            let mcc_server = self.mcc_server.clone();
            let root_clone = root.clone();
            tokio::spawn(async move {
                // Wait for mcc server to be ready
                let max_wait = 50; // 5 seconds max
                for _ in 0..max_wait {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let server_guard = mcc_server.read().await;
                    if let Some(server) = server_guard.as_ref() {
                        if server.is_connected() {
                            if let Some(client) = server.client() {
                                if let Some(config) = ProjectConfig::load_from(&root_clone) {
                                    debug!(
                                        "Auto-loading {} dependencies...",
                                        config.dependency_names().len()
                                    );
                                    for lib_name in config.dependency_names() {
                                        let _ = client.lib_load(lib_name).await;
                                    }
                                    let entry = config.entry_path(&root_clone);
                                    let _ = client.load_project(&entry).await;
                                }
                            }
                            break;
                        }
                    }
                }
            });
        }
    }

    async fn execute_command(&self, _: ExecuteCommandParams) -> Result<Option<serde_json::Value>> {
        debug!("execute_command");
        Ok(None)
    }

    // Phase 5: document links disabled — hover handles this now
    async fn document_link(
        &self,
        _params: DocumentLinkParams,
    ) -> Result<Option<Vec<DocumentLink>>> {
        Ok(None)
    }

    // Phase 5: hover
    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let span = tracing::debug_span!("hover");
        let _guard = span.enter();
        Ok(crate::features::hover::resolve(&self.state, &params))
    }
}
