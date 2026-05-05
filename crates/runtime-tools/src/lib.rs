use async_trait::async_trait;
use runtime_core::{ProcessManager, RuntimeError, TeamCommsService, ToolGateway, WorktreeService};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessManagerConfig {
    pub enabled: bool,
    pub max_concurrent: usize,
    pub default_timeout_ms: u64,
    pub max_output_bytes_per_process: usize,
    pub allow_shell: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamCommsConfig {
    pub enabled: bool,
    pub max_pending_deliveries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeServiceConfig {
    pub enabled: bool,
    pub root_dir: String,
    pub init_script_path: String,
    pub deletion_policy_default: String,
}

#[derive(Debug, Default)]
pub struct StubToolGateway;

#[derive(Debug)]
pub struct StubProcessManager {
    config: ProcessManagerConfig,
}

#[derive(Debug)]
pub struct StubTeamCommsService {
    config: TeamCommsConfig,
}

#[derive(Debug)]
pub struct StubWorktreeService {
    config: WorktreeServiceConfig,
}

impl StubProcessManager {
    pub fn new(config: ProcessManagerConfig) -> Self {
        Self { config }
    }
}

impl StubTeamCommsService {
    pub fn new(config: TeamCommsConfig) -> Self {
        Self { config }
    }
}

impl StubWorktreeService {
    pub fn new(config: WorktreeServiceConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ToolGateway for StubToolGateway {
    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

#[async_trait]
impl ProcessManager for StubProcessManager {
    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        let _enabled = self.config.enabled;
        Ok(())
    }
}

#[async_trait]
impl TeamCommsService for StubTeamCommsService {
    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        if self.config.enabled {
            return Ok(());
        }
        Err(RuntimeError::Bootstrap(
            "team comms service is disabled".to_string(),
        ))
    }
}

#[async_trait]
impl WorktreeService for StubWorktreeService {
    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        let _enabled = self.config.enabled;
        Ok(())
    }
}
