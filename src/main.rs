use mcodels::Backend;
use std::sync::Mutex;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let log_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("log.txt");
    // Truncate log file for fresh session
    let _ = std::fs::write(&log_path, "");

    // Open log file; fall back to stderr if unavailable.
    // stderr may interfere with VS Code LSP client, but it's better than crashing.
    let writer: Mutex<Box<dyn std::io::Write + Send>> = Mutex::new(
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(f) => Box::new(f),
            Err(e) => {
                eprintln!("warning: cannot open log.txt: {e}, falling back to stderr");
                Box::new(std::io::stderr())
            }
        },
    );

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::build(Backend::new).finish();

    Server::new(stdin, stdout, socket).serve(service).await;
}
