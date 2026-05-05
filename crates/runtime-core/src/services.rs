use async_trait::async_trait;

use crate::RuntimeError;

#[async_trait]
pub trait RuntimeStore: Send + Sync {
    async fn initialize(&self) -> Result<(), RuntimeError>;

    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}

#[async_trait]
pub trait ToolGateway: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}

#[async_trait]
pub trait TeamCommsService: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}

#[async_trait]
pub trait WorktreeService: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}
