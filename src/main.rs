use mcodels::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let log_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("log.txt");
    // Truncate log file for fresh session
    let _ = std::fs::write(&log_path, "");

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("failed to open log file");

    // Only write to log.txt; DO NOT write to stderr/stdout
    // (stderr output can interfere with VS Code LSP client)
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(file)
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
