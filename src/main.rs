use mcodels::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    // Initialize tracing-subscriber, explicitly use stderr to avoid polluting LSP stdout
    // (default fmt() is also stderr, but being explicit is safer — prevents issues if default
    // behavior changes or another crate registers a subscriber first)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
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
