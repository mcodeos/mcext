use mcodels::Backend;
use std::backtrace::Backtrace;
use std::io::Write as _;
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

    // Panic hook: dump the panic message + backtrace into log.txt (and stderr).
    // The default handler only writes to stderr, which the VS Code LSP client
    // swallows, so crashes would otherwise leave no trace. Force-capturing the
    // backtrace makes this work even without RUST_BACKTRACE set. The hook
    // reopens the log file itself rather than sharing the tracing writer
    // (tracing-subscriber's MakeWriter does not accept an Arc wrapper).
    let panic_log_path = log_path.clone();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let msg = format!(
            "=== PANIC ===\nthread '{}' panicked at {}:\n{}\n\nBacktrace (forced capture):\n{}\n",
            thread.name().unwrap_or("<unnamed>"),
            info.location()
                .map(|loc| loc.to_string())
                .unwrap_or_else(|| "<no location>".to_string()),
            info.payload_as_str()
                .unwrap_or("<non-string panic payload>"),
            Backtrace::force_capture(),
        );
        // Write to log.txt; never panic inside the panic hook itself.
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log_path)
        {
            let _ = f.write_all(msg.as_bytes());
        }
        // Also emit to stderr so terminal runs see it immediately.
        eprint!("{msg}");
    }));

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
