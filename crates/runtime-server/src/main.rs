use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use runtime_server::{
    bootstrap_runtime, build_router, write_openapi_artifact, AppState, AuthBootstrapSource,
    RuntimeServerConfig,
};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = parse_cli()?;
    if let CliCommand::WriteOpenApi { path } = cli {
        write_openapi_artifact(path.as_path())
            .with_context(|| format!("failed to write {}", path.display()))?;
        tracing::info!(path = %path.display(), "wrote generated OpenAPI artifact");
        return Ok(());
    }

    let CliCommand::Serve { config_path } = cli else {
        return Err(anyhow!("unsupported CLI command"));
    };
    let config = RuntimeServerConfig::load(config_path.as_deref())?;
    let bootstrapped = bootstrap_runtime(config).await?;

    let token_source = describe_auth_source(&bootstrapped.auth.source);
    tracing::info!(
        bind = %bootstrapped.bind_address,
        public_base_url = %bootstrapped.public_base_url,
        provider_count = bootstrapped.app.provider_registry.len(),
        token_source = %token_source,
        "runtime bootstrapped"
    );

    let state = AppState {
        app: bootstrapped.app,
        runtime: bootstrapped.runtime,
        bearer_token: bootstrapped.auth.bearer_token,
        public_base_url: bootstrapped.public_base_url,
        startup_recovery: Arc::new(bootstrapped.startup_recovery),
    };

    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(&bootstrapped.bind_address)
        .await
        .with_context(|| format!("failed to bind {}", bootstrapped.bind_address))?;

    tracing::info!("gg-runtime-server listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server failed")?;

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();
}

enum CliCommand {
    Serve { config_path: Option<PathBuf> },
    WriteOpenApi { path: PathBuf },
}

fn parse_cli() -> Result<CliCommand> {
    let mut args = std::env::args().skip(1);
    let mut config_path = None;
    let mut write_openapi_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--config requires a path argument"))?;
                config_path = Some(PathBuf::from(value));
            }
            "--write-openapi" => {
                let path = args.next().map(PathBuf::from).unwrap_or_else(|| {
                    PathBuf::from("openapi").join("runtime-server-openapi.yaml")
                });
                write_openapi_path = Some(path);
            }
            other => {
                return Err(anyhow!("unknown argument: {other}"));
            }
        }
    }
    if let Some(path) = write_openapi_path {
        return Ok(CliCommand::WriteOpenApi { path });
    }
    Ok(CliCommand::Serve { config_path })
}

fn describe_auth_source(source: &AuthBootstrapSource) -> String {
    match source {
        AuthBootstrapSource::InlineConfig => "inline-config".to_string(),
        AuthBootstrapSource::TokenFileExisting { path } => {
            format!("token-file-existing:{}", path.display())
        }
        AuthBootstrapSource::TokenFileCreated { path } => {
            format!("token-file-created:{}", path.display())
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            let _ = stream.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
