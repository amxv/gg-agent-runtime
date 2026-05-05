use std::path::PathBuf;

use async_trait::async_trait;
use runtime_core::{ProviderKind, ProviderMetadata, RuntimeError, RuntimeProvider};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeProviderConfig {
    pub enabled: bool,
    pub config_dir: PathBuf,
    pub bridge_command: String,
    pub max_bridges: usize,
    pub max_sessions_per_bridge: usize,
}

#[derive(Debug)]
pub struct ClaudeProviderStub {
    config: ClaudeProviderConfig,
}

impl ClaudeProviderStub {
    pub fn new(config: ClaudeProviderConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl RuntimeProvider for ClaudeProviderStub {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Claude
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            kind: ProviderKind::Claude,
            display_name: "Claude".to_string(),
            enabled: self.config.enabled,
        }
    }

    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        if self.config.enabled {
            return Ok(());
        }
        Err(RuntimeError::Bootstrap(
            "claude provider disabled".to_string(),
        ))
    }
}
