use std::path::PathBuf;

use async_trait::async_trait;
use runtime_core::{ProviderKind, ProviderMetadata, RuntimeError, RuntimeProvider};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexProviderConfig {
    pub enabled: bool,
    pub home_dir: PathBuf,
    pub max_transports: usize,
    pub max_sessions_per_transport: usize,
}

#[derive(Debug)]
pub struct CodexProviderStub {
    config: CodexProviderConfig,
}

impl CodexProviderStub {
    pub fn new(config: CodexProviderConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl RuntimeProvider for CodexProviderStub {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Codex
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            kind: ProviderKind::Codex,
            display_name: "Codex".to_string(),
            enabled: self.config.enabled,
        }
    }

    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        if self.config.enabled {
            return Ok(());
        }
        Err(RuntimeError::Bootstrap(
            "codex provider disabled".to_string(),
        ))
    }
}
