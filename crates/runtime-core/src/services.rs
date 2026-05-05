use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvokeRequest {
    pub namespace: Option<String>,
    pub tool_name: String,
    pub caller_session_id: String,
    pub invocation_id: Option<String>,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRunRequest {
    pub caller_session_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessListRequest {
    pub caller_session_id: Option<String>,
    pub include_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessGetRequest {
    pub process_id: String,
    pub caller_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLogReadRequest {
    pub process_id: String,
    pub caller_session_id: Option<String>,
    pub stream: Option<String>,
    pub head_lines: Option<usize>,
    pub tail_lines: Option<usize>,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessKillRequest {
    pub process_id: String,
    pub caller_session_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSummary {
    pub process_id: String,
    pub session_id: Option<String>,
    pub pid: Option<i64>,
    pub status: String,
    pub command: Value,
    pub cwd: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessDetails {
    pub process: ProcessSummary,
    pub exit_code: Option<i64>,
    pub signal: Option<i64>,
    pub timeout_ms: Option<i64>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLogsChunk {
    pub process_id: String,
    pub stream: String,
    pub content: String,
    pub head_lines: usize,
    pub tail_lines: usize,
    pub truncated: bool,
    pub bytes: usize,
}

#[async_trait]
pub trait ToolGateway: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;

    async fn invoke_tool(&self, request: ToolInvokeRequest) -> Result<Value, RuntimeError>;

    async fn capabilities(&self) -> Result<Value, RuntimeError>;
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;

    async fn run_process(&self, request: ProcessRunRequest)
        -> Result<ProcessDetails, RuntimeError>;

    async fn list_processes(
        &self,
        request: ProcessListRequest,
    ) -> Result<Vec<ProcessSummary>, RuntimeError>;

    async fn get_process(&self, request: ProcessGetRequest)
        -> Result<ProcessDetails, RuntimeError>;

    async fn read_process_logs(
        &self,
        request: ProcessLogReadRequest,
    ) -> Result<Vec<ProcessLogsChunk>, RuntimeError>;

    async fn kill_process(
        &self,
        request: ProcessKillRequest,
    ) -> Result<ProcessDetails, RuntimeError>;

    async fn replay_events(
        &self,
        process_id: String,
        caller_session_id: Option<String>,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<RuntimeEventRecord>, RuntimeError>;

    fn subscribe_events(&self) -> broadcast::Receiver<RuntimeEventRecord>;
}

#[async_trait]
pub trait TeamCommsService: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}

#[async_trait]
pub trait WorktreeService: Send + Sync {
    async fn healthcheck(&self) -> Result<(), RuntimeError>;
}
