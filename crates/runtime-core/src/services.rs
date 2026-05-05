use async_trait::async_trait;

use crate::{
    ApprovalRecord, ManagedWorktreeClaimRecord, ManagedWorktreeRecord, NewRuntimeEvent,
    ProcessRecord, RuntimeError, RuntimeEventRecord, RuntimeEventScope, RuntimeHydratedState,
    SessionRecord, TeamDeliveryRecord, TeamMemberRecord, TeamMessageRecord, TeamRecord, TurnRecord,
};

#[async_trait]
pub trait RuntimeStore: Send + Sync {
    async fn initialize(&self) -> Result<(), RuntimeError>;

    async fn healthcheck(&self) -> Result<(), RuntimeError>;

    fn append_runtime_event(
        &self,
        event: &NewRuntimeEvent,
    ) -> Result<RuntimeEventRecord, RuntimeError>;

    fn list_runtime_events(
        &self,
        scope: Option<(RuntimeEventScope, &str)>,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<RuntimeEventRecord>, RuntimeError>;

    fn upsert_session(&self, record: &SessionRecord) -> Result<(), RuntimeError>;

    fn upsert_turn(&self, record: &TurnRecord) -> Result<(), RuntimeError>;

    fn upsert_approval(&self, record: &ApprovalRecord) -> Result<(), RuntimeError>;

    fn upsert_team(&self, record: &TeamRecord) -> Result<(), RuntimeError>;

    fn upsert_team_member(&self, record: &TeamMemberRecord) -> Result<(), RuntimeError>;

    fn upsert_team_message(&self, record: &TeamMessageRecord) -> Result<(), RuntimeError>;

    fn upsert_team_delivery(&self, record: &TeamDeliveryRecord) -> Result<(), RuntimeError>;

    fn upsert_managed_worktree(&self, record: &ManagedWorktreeRecord) -> Result<(), RuntimeError>;

    fn upsert_managed_worktree_claim(
        &self,
        record: &ManagedWorktreeClaimRecord,
    ) -> Result<(), RuntimeError>;

    fn upsert_process(&self, record: &ProcessRecord) -> Result<(), RuntimeError>;

    fn hydrate_runtime_state(&self) -> Result<RuntimeHydratedState, RuntimeError>;
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
