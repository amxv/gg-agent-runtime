use std::sync::Arc;

use anyhow::{Context, Result};
use runtime_core::{
    EventQueueLimits, ProcessLimits, ProviderRegistry, RuntimeApp, RuntimeServices,
    RuntimeSessionManager, WorktreeSettings,
};
use runtime_provider_claude::{ClaudeProviderConfig, ClaudeProviderStub};
use runtime_provider_codex::{copy_codex_auth_file, CodexProvider, CodexProviderConfig};
use runtime_store_sqlite::{SqliteRuntimeStore, SqliteStoreConfig};
use runtime_tools::{
    ProcessManagerConfig, StubProcessManager, StubTeamCommsService, StubToolGateway,
    StubWorktreeService, TeamCommsConfig, WorktreeServiceConfig,
};

use crate::config::{ResolvedAuth, RuntimeServerConfig};

#[derive(Clone)]
pub struct BootstrappedRuntime {
    pub app: Arc<RuntimeApp>,
    pub runtime: Arc<RuntimeSessionManager>,
    pub auth: ResolvedAuth,
    pub bind_address: String,
    pub public_base_url: String,
}

pub async fn bootstrap_runtime(config: RuntimeServerConfig) -> Result<BootstrappedRuntime> {
    config.ensure_data_dirs()?;
    let auth = config.bootstrap_auth()?;

    let sqlite_path = config.resolve_sqlite_path();
    let store = Arc::new(SqliteRuntimeStore::new(SqliteStoreConfig {
        database_path: sqlite_path,
    }));

    let codex_home = config.resolve_provider_dir("codex").join("home");
    let codex_provider = Arc::new(CodexProvider::new(CodexProviderConfig {
        enabled: config.providers.codex.enabled,
        home_dir: codex_home.clone(),
        max_transports: config.providers.codex.max_instances,
        max_sessions_per_transport: config.providers.codex.max_sessions_per_instance,
    }));

    if config.providers.codex.enabled {
        let default_auth_source = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map(|path| path.join(".gg").join("codex").join("auth.json"));
        if let Some(source_auth_path) = default_auth_source.as_ref() {
            if source_auth_path.exists() {
                copy_codex_auth_file(source_auth_path, codex_home.as_path())
                    .context("failed to stage codex auth.json into runtime provider home")?;
            }
        }
    }

    let claude_provider = Arc::new(ClaudeProviderStub::new(ClaudeProviderConfig {
        enabled: config.providers.claude.enabled,
        config_dir: config.resolve_provider_dir("claude").join("config"),
        bridge_command: "claude-bridge".to_string(),
        max_bridges: config.providers.claude.max_instances,
        max_sessions_per_bridge: config.providers.claude.max_sessions_per_instance,
    }));

    let mut provider_registry = ProviderRegistry::new();
    if config.providers.codex.enabled {
        provider_registry
            .register(codex_provider)
            .context("failed to register codex provider")?;
    }
    if config.providers.claude.enabled {
        provider_registry
            .register(claude_provider)
            .context("failed to register claude provider")?;
    }

    let services = RuntimeServices {
        store: store.clone(),
        tool_gateway: Arc::new(StubToolGateway),
        process_manager: Arc::new(StubProcessManager::new(ProcessManagerConfig {
            enabled: config.processes.enabled,
            max_concurrent: config.processes.max_concurrent,
            default_timeout_ms: config.processes.default_timeout_ms,
            max_output_bytes_per_process: config.processes.max_output_bytes_per_process,
            allow_shell: config.processes.allow_shell,
        })),
        team_comms: Arc::new(StubTeamCommsService::new(TeamCommsConfig {
            enabled: true,
            max_pending_deliveries: 10_000,
        })),
        worktrees: Arc::new(StubWorktreeService::new(WorktreeServiceConfig {
            enabled: config.worktrees.enabled,
            root_dir: config.resolve_worktree_root().display().to_string(),
            init_script_path: config.worktrees.init_script_path.display().to_string(),
            deletion_policy_default: config.worktrees.deletion_policy_default.clone(),
        })),
    };

    let provider_registry = Arc::new(provider_registry);
    let app = RuntimeApp::new(
        provider_registry.clone(),
        services,
        EventQueueLimits {
            live_queue_capacity: config.events.live_queue_capacity,
            critical_queue_capacity: config.events.critical_queue_capacity,
            team_queue_capacity: config.events.team_queue_capacity,
        },
        ProcessLimits {
            max_concurrent: config.processes.max_concurrent,
            default_timeout_ms: config.processes.default_timeout_ms,
            max_output_bytes_per_process: config.processes.max_output_bytes_per_process,
        },
        WorktreeSettings {
            enabled: config.worktrees.enabled,
            root_dir: config.resolve_worktree_root().display().to_string(),
            init_script_path: config.worktrees.init_script_path.display().to_string(),
            deletion_policy_default: config.worktrees.deletion_policy_default.clone(),
        },
    )
    .context("failed to build runtime app composition")?;

    let app = Arc::new(app);
    app.initialize()
        .await
        .context("runtime initialization failed")?;
    let runtime = Arc::new(
        RuntimeSessionManager::new(
            store.clone(),
            provider_registry,
            config.events.live_queue_capacity,
        )
        .context("failed to initialize runtime session manager")?,
    );

    Ok(BootstrappedRuntime {
        app,
        runtime,
        auth,
        bind_address: config.server.bind_address,
        public_base_url: config.server.public_base_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bootstrap_fails_when_all_providers_disabled() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        config.providers.codex.enabled = false;
        config.providers.claude.enabled = false;

        let result = bootstrap_runtime(config).await;
        assert!(result.is_err(), "bootstrap should fail");
    }

    #[tokio::test]
    async fn bootstrap_registers_enabled_providers() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        config.providers.codex.enabled = true;
        config.providers.claude.enabled = false;

        let runtime = bootstrap_runtime(config).await.expect("bootstrap");
        assert_eq!(runtime.app.provider_registry.len(), 1);
    }

    #[tokio::test]
    async fn bootstrap_succeeds_when_processes_and_worktrees_are_disabled() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        config.processes.enabled = false;
        config.worktrees.enabled = false;

        let runtime = bootstrap_runtime(config).await.expect("bootstrap");
        assert_eq!(runtime.app.provider_registry.len(), 2);
        assert!(!runtime.app.worktree_settings.enabled);
    }

    #[tokio::test]
    async fn bootstrap_wires_worktree_deletion_policy_default() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut config = RuntimeServerConfig::default();
        config.data.root_dir = temp_dir.path().to_path_buf();
        config.worktrees.deletion_policy_default = "retain_on_last_claim".to_string();

        let runtime = bootstrap_runtime(config).await.expect("bootstrap");
        assert_eq!(
            runtime.app.worktree_settings.deletion_policy_default,
            "retain_on_last_claim"
        );
    }
}
