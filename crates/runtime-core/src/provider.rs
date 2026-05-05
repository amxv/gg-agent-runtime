use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::RuntimeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Codex,
    Claude,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetadata {
    pub kind: ProviderKind,
    pub display_name: String,
    pub enabled: bool,
}

#[async_trait]
pub trait RuntimeProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    fn metadata(&self) -> ProviderMetadata;

    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}
