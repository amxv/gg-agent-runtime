use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    ProcessManager, ProviderRegistry, RuntimeError, RuntimeStore, TeamCommsService, ToolGateway,
    WorktreeService,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventQueueLimits {
    pub live_queue_capacity: usize,
    pub critical_queue_capacity: usize,
    pub team_queue_capacity: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLimits {
    pub max_concurrent: usize,
    pub default_timeout_ms: u64,
    pub max_output_bytes_per_process: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeSettings {
    pub enabled: bool,
    pub root_dir: String,
    pub init_script_path: String,
    pub deletion_policy_default: String,
}

#[derive(Clone)]
pub struct RuntimeServices {
    pub store: Arc<dyn RuntimeStore>,
    pub tool_gateway: Arc<dyn ToolGateway>,
    pub process_manager: Arc<dyn ProcessManager>,
    pub team_comms: Arc<dyn TeamCommsService>,
    pub worktrees: Arc<dyn WorktreeService>,
}

#[derive(Clone)]
pub struct RuntimeApp {
    pub provider_registry: Arc<ProviderRegistry>,
    pub services: RuntimeServices,
    pub event_queue_limits: EventQueueLimits,
    pub process_limits: ProcessLimits,
    pub worktree_settings: WorktreeSettings,
}

impl RuntimeApp {
    pub fn new(
        provider_registry: Arc<ProviderRegistry>,
        services: RuntimeServices,
        event_queue_limits: EventQueueLimits,
        process_limits: ProcessLimits,
        worktree_settings: WorktreeSettings,
    ) -> Result<Self, RuntimeError> {
        if provider_registry.is_empty() {
            return Err(RuntimeError::Configuration(
                "at least one provider must be configured".to_string(),
            ));
        }

        if event_queue_limits.live_queue_capacity == 0
            || event_queue_limits.critical_queue_capacity == 0
            || event_queue_limits.team_queue_capacity == 0
        {
            return Err(RuntimeError::Configuration(
                "event queue capacities must be greater than zero".to_string(),
            ));
        }

        if process_limits.max_concurrent == 0 {
            return Err(RuntimeError::Configuration(
                "process max_concurrent must be greater than zero".to_string(),
            ));
        }

        Ok(Self {
            provider_registry,
            services,
            event_queue_limits,
            process_limits,
            worktree_settings,
        })
    }

    pub async fn initialize(&self) -> Result<(), RuntimeError> {
        self.services.store.initialize().await?;
        self.services.store.healthcheck().await?;
        self.services.tool_gateway.healthcheck().await?;
        self.services.process_manager.healthcheck().await?;
        self.services.team_comms.healthcheck().await?;
        self.services.worktrees.healthcheck().await?;
        Ok(())
    }
}
