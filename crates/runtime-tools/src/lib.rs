use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use runtime_core::{
    NewRuntimeEvent, ProcessDetails, ProcessGetRequest, ProcessKillRequest, ProcessListRequest,
    ProcessLogReadRequest, ProcessLogsChunk, ProcessManager, ProcessRecord, ProcessRunRequest,
    ProcessSummary, RuntimeError, RuntimeEventCriticality, RuntimeEventScope, RuntimeStore,
    TeamCommsService, ToolGateway, ToolInvokeRequest, WorktreeService,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex, OwnedSemaphorePermit, RwLock, Semaphore};

const GG_PROCESS_RUN: &str = "gg_process_run";
const GG_PROCESS_STATUS: &str = "gg_process_status";
const GG_PROCESS_LOGS: &str = "gg_process_logs";
const GG_PROCESS_KILL: &str = "gg_process_kill";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessManagerConfig {
    pub enabled: bool,
    pub max_concurrent: usize,
    pub default_timeout_ms: u64,
    pub max_output_bytes_per_process: usize,
    pub allow_shell: bool,
    pub completed_retention_ms: u64,
    pub output_event_sample_bytes: usize,
    pub log_dir: PathBuf,
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

pub struct RuntimeProcessManager {
    store: Arc<dyn RuntimeStore>,
    config: ProcessManagerConfig,
    semaphore: Arc<Semaphore>,
    next_process_id: Arc<AtomicU64>,
    next_event_id: Arc<AtomicU64>,
    processes: Arc<RwLock<HashMap<String, Arc<ManagedProcess>>>>,
    event_tx: broadcast::Sender<runtime_core::RuntimeEventRecord>,
}

#[derive(Debug)]
struct ManagedProcess {
    record: Mutex<ProcessRecord>,
    child: Mutex<Option<tokio::process::Child>>,
    stdout_bytes: Mutex<usize>,
    stderr_bytes: Mutex<usize>,
    stdout_truncated: Mutex<bool>,
    stderr_truncated: Mutex<bool>,
    kill_requested: Mutex<bool>,
    timed_out: Mutex<bool>,
}

impl ManagedProcess {
    fn new(record: ProcessRecord, child: Option<tokio::process::Child>) -> Self {
        Self {
            record: Mutex::new(record),
            child: Mutex::new(child),
            stdout_bytes: Mutex::new(0),
            stderr_bytes: Mutex::new(0),
            stdout_truncated: Mutex::new(false),
            stderr_truncated: Mutex::new(false),
            kill_requested: Mutex::new(false),
            timed_out: Mutex::new(false),
        }
    }
}

impl RuntimeProcessManager {
    pub async fn new(
        store: Arc<dyn RuntimeStore>,
        config: ProcessManagerConfig,
    ) -> Result<Arc<Self>, RuntimeError> {
        let _ = store.initialize().await;
        std::fs::create_dir_all(&config.log_dir).map_err(|error| {
            RuntimeError::Bootstrap(format!(
                "failed to create process log dir {}: {error}",
                config.log_dir.display()
            ))
        })?;

        let hydrated = store.hydrate_runtime_state()?;
        let mut processes = HashMap::new();
        let mut max_seq = 0_u64;
        let (event_tx, _) = broadcast::channel(16_384);

        for mut record in hydrated.processes {
            if let Some(seq) = parse_process_sequence(record.id.as_str()) {
                max_seq = max_seq.max(seq);
            }
            if record.status == "running" || record.status == "queued" {
                record.status = "failed".to_string();
                record.ended_at = Some(now_ms());
            }
            processes.insert(
                record.id.clone(),
                Arc::new(ManagedProcess::new(record, None)),
            );
        }

        Ok(Arc::new(Self {
            store,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent.max(1))),
            config,
            next_process_id: Arc::new(AtomicU64::new(max_seq + 1)),
            next_event_id: Arc::new(AtomicU64::new(1)),
            processes: Arc::new(RwLock::new(processes)),
            event_tx,
        }))
    }

    async fn append_process_event(
        &self,
        process_id: &str,
        session_id: Option<String>,
        kind: &str,
        criticality: RuntimeEventCriticality,
        payload: Value,
    ) {
        let event_id = format!(
            "evt_proc_{}_{}",
            process_id,
            self.next_event_id.fetch_add(1, Ordering::Relaxed)
        );
        if let Ok(record) = self.store.append_runtime_event(&NewRuntimeEvent {
            event_id,
            scope: RuntimeEventScope::Process,
            scope_id: process_id.to_string(),
            session_id,
            team_id: None,
            turn_id: None,
            kind: kind.to_string(),
            criticality,
            payload,
            provider: None,
            provider_seq: None,
            created_at: now_ms(),
        }) {
            let _ = self.event_tx.send(record);
        }
    }

    async fn process_from_id(&self, process_id: &str) -> Result<Arc<ManagedProcess>, RuntimeError> {
        let processes = self.processes.read().await;
        processes
            .get(process_id)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound(format!("process {process_id}")))
    }

    async fn process_from_pid(&self, pid: i64) -> Result<Arc<ManagedProcess>, RuntimeError> {
        let processes = self.processes.read().await;
        for process in processes.values() {
            let record = process.record.lock().await;
            if record.pid == Some(pid) {
                return Ok(Arc::clone(process));
            }
        }
        Err(RuntimeError::NotFound(format!("process pid {pid}")))
    }

    async fn ensure_ownership(
        &self,
        process: &ManagedProcess,
        caller_session_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let Some(caller_session_id) = caller_session_id else {
            return Ok(());
        };
        let record = process.record.lock().await;
        if record.session_id.as_deref() == Some(caller_session_id) {
            return Ok(());
        }
        Err(RuntimeError::InvalidState(format!(
            "process {} belongs to a different session",
            record.id
        )))
    }

    async fn cleanup_expired_terminal(&self) {
        if self.config.completed_retention_ms == 0 {
            return;
        }
        let now = now_ms();
        let retention_ms = self.config.completed_retention_ms as i64;

        let snapshots = {
            let processes = self.processes.read().await;
            let mut rows = Vec::with_capacity(processes.len());
            for (id, process) in processes.iter() {
                let record = process.record.lock().await;
                rows.push((id.clone(), record.status.clone(), record.ended_at));
            }
            rows
        };

        let mut to_remove = Vec::new();
        for (id, status, ended_at) in snapshots {
            if !matches!(
                status.as_str(),
                "completed" | "failed" | "timed_out" | "killed"
            ) {
                continue;
            }
            if let Some(ended_at) = ended_at {
                if now.saturating_sub(ended_at) >= retention_ms {
                    to_remove.push(id);
                }
            }
        }

        if !to_remove.is_empty() {
            let mut processes = self.processes.write().await;
            for id in to_remove {
                processes.remove(id.as_str());
            }
        }
    }

    async fn list_process_entries(
        &self,
        caller_session_id: Option<&str>,
        include_completed: bool,
    ) -> Result<Vec<ProcessSummary>, RuntimeError> {
        self.cleanup_expired_terminal().await;

        let processes = self.processes.read().await;
        let mut rows = Vec::new();
        for process in processes.values() {
            let record = process.record.lock().await;
            if let Some(caller_session_id) = caller_session_id {
                if record.session_id.as_deref() != Some(caller_session_id) {
                    continue;
                }
            }
            if !include_completed
                && matches!(
                    record.status.as_str(),
                    "completed" | "failed" | "timed_out" | "killed"
                )
            {
                continue;
            }
            rows.push(ProcessSummary {
                process_id: record.id.clone(),
                session_id: record.session_id.clone(),
                pid: record.pid,
                status: record.status.clone(),
                command: record.command.clone(),
                cwd: record.cwd.clone(),
                started_at: record.started_at,
                ended_at: record.ended_at,
            });
        }
        rows.sort_by(|left, right| right.started_at.cmp(&left.started_at));
        Ok(rows)
    }

    async fn detail_from_process(process: &ManagedProcess) -> ProcessDetails {
        let record = process.record.lock().await;
        let stdout_bytes = *process.stdout_bytes.lock().await;
        let stderr_bytes = *process.stderr_bytes.lock().await;
        let stdout_truncated = *process.stdout_truncated.lock().await;
        let stderr_truncated = *process.stderr_truncated.lock().await;

        ProcessDetails {
            process: ProcessSummary {
                process_id: record.id.clone(),
                session_id: record.session_id.clone(),
                pid: record.pid,
                status: record.status.clone(),
                command: record.command.clone(),
                cwd: record.cwd.clone(),
                started_at: record.started_at,
                ended_at: record.ended_at,
            },
            exit_code: record.exit_code,
            signal: record.signal,
            timeout_ms: record.timeout_ms,
            stdout_path: record.stdout_path.clone(),
            stderr_path: record.stderr_path.clone(),
            stdout_bytes,
            stderr_bytes,
            stdout_truncated,
            stderr_truncated,
        }
    }

    async fn run_lifecycle(
        self: Arc<Self>,
        process: Arc<ManagedProcess>,
        _spawn_permit: OwnedSemaphorePermit,
    ) {
        let (mut child, process_id, session_id, stdout_path, stderr_path, timeout_ms) = {
            let mut child_lock = process.child.lock().await;
            let Some(child) = child_lock.take() else {
                return;
            };
            let record = process.record.lock().await;
            (
                child,
                record.id.clone(),
                record.session_id.clone(),
                record
                    .stdout_path
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        self.config
                            .log_dir
                            .join(format!("{}.stdout.log", record.id))
                    }),
                record
                    .stderr_path
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        self.config
                            .log_dir
                            .join(format!("{}.stderr.log", record.id))
                    }),
                record.timeout_ms.map(|value| value as u64),
            )
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_task = stdout.map(|stream| {
            tokio::spawn(Self::pump_stream(
                Arc::clone(&self),
                Arc::clone(&process),
                process_id.clone(),
                session_id.clone(),
                "stdout",
                stream,
                stdout_path.clone(),
            ))
        });
        let stderr_task = stderr.map(|stream| {
            tokio::spawn(Self::pump_stream(
                Arc::clone(&self),
                Arc::clone(&process),
                process_id.clone(),
                session_id.clone(),
                "stderr",
                stream,
                stderr_path.clone(),
            ))
        });

        let timeout = timeout_ms.unwrap_or(self.config.default_timeout_ms).max(1);
        let wait_result = tokio::select! {
            result = child.wait() => result.map(|status| (status.code(), exit_status_signal(&status))),
            _ = tokio::time::sleep(std::time::Duration::from_millis(timeout)) => {
                {
                    let mut timed_out = process.timed_out.lock().await;
                    *timed_out = true;
                }
                let _ = child.start_kill();
                child.wait().await.map(|status| (status.code(), exit_status_signal(&status)))
            }
        };

        if let Some(task) = stdout_task {
            let _ = task.await;
        }
        if let Some(task) = stderr_task {
            let _ = task.await;
        }

        let (status, exit_code, signal) = match wait_result {
            Ok((code, signal)) => {
                let timed_out = *process.timed_out.lock().await;
                let killed = *process.kill_requested.lock().await;
                if timed_out {
                    ("timed_out".to_string(), code, signal)
                } else if killed {
                    ("killed".to_string(), code, signal)
                } else if code == Some(0) {
                    ("completed".to_string(), code, signal)
                } else {
                    ("failed".to_string(), code, signal)
                }
            }
            Err(error) => ("failed".to_string(), None, error.raw_os_error()),
        };

        {
            let mut record = process.record.lock().await;
            record.status = status.clone();
            record.exit_code = exit_code.map(i64::from);
            record.signal = signal.map(i64::from);
            record.ended_at = Some(now_ms());
            let _ = self.store.upsert_process(&record);
        }

        let event_kind = match status.as_str() {
            "completed" => "process.completed",
            "timed_out" => "process.timed_out",
            "killed" => "process.killed",
            _ => "process.failed",
        };

        self.append_process_event(
            process_id.as_str(),
            session_id,
            event_kind,
            RuntimeEventCriticality::Critical,
            json!({
                "process_id": process_id,
                "status": status,
                "exit_code": exit_code,
                "signal": signal,
            }),
        )
        .await;
    }

    async fn pump_stream<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
        manager: Arc<Self>,
        process: Arc<ManagedProcess>,
        process_id: String,
        session_id: Option<String>,
        stream_name: &'static str,
        mut reader: R,
        path: PathBuf,
    ) {
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            Ok(file) => file,
            Err(_) => return,
        };

        let max_bytes = manager.config.max_output_bytes_per_process;
        let sample_bytes = manager.config.output_event_sample_bytes.max(1);

        let mut buffer = vec![0_u8; 8192];
        let mut emitted_budget = 0_usize;

        loop {
            let read = match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(size) => size,
                Err(_) => break,
            };
            let chunk = &buffer[..read];

            let (bytes_written, truncated_now) = {
                let bytes_counter = if stream_name == "stdout" {
                    &process.stdout_bytes
                } else {
                    &process.stderr_bytes
                };
                let mut used = bytes_counter.lock().await;
                let remaining = max_bytes.saturating_sub(*used);
                let to_write = remaining.min(chunk.len());
                let truncated_now = to_write < chunk.len();

                if to_write > 0 {
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk[..to_write]).await;
                    *used += to_write;
                }

                if truncated_now {
                    let truncated_flag = if stream_name == "stdout" {
                        &process.stdout_truncated
                    } else {
                        &process.stderr_truncated
                    };
                    let mut truncated = truncated_flag.lock().await;
                    *truncated = true;
                }

                (to_write, truncated_now)
            };

            emitted_budget = emitted_budget.saturating_add(read);
            if emitted_budget >= sample_bytes || truncated_now {
                emitted_budget = 0;
                manager
                    .append_process_event(
                        process_id.as_str(),
                        session_id.clone(),
                        "process.output",
                        RuntimeEventCriticality::Droppable,
                        json!({
                            "process_id": process_id,
                            "stream": stream_name,
                            "bytes_seen": read,
                            "bytes_written": bytes_written,
                            "truncated": truncated_now,
                        }),
                    )
                    .await;
            }
        }
    }
}

#[async_trait]
impl ProcessManager for RuntimeProcessManager {
    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn run_process(
        &self,
        request: ProcessRunRequest,
    ) -> Result<ProcessDetails, RuntimeError> {
        if !self.config.enabled {
            return Err(RuntimeError::Unsupported(
                "gg_process tools are disabled".to_string(),
            ));
        }

        let command = request.command.trim();
        if command.is_empty() {
            return Err(RuntimeError::InvalidState(
                "command cannot be empty".to_string(),
            ));
        }
        let spawn_permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::InvalidState("process semaphore closed".to_string()))?;

        let process_sequence = self.next_process_id.fetch_add(1, Ordering::Relaxed);
        let process_id = format!("proc_{process_sequence}");
        let stdout_path = self.config.log_dir.join(format!("{process_id}.stdout.log"));
        let stderr_path = self.config.log_dir.join(format!("{process_id}.stderr.log"));

        let cwd = request
            .cwd
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let mut proc = if self.config.allow_shell {
            let mut process = Command::new("sh");
            process.arg("-lc");
            process.arg(command);
            process
        } else {
            let mut split = command.split_whitespace();
            let executable = split
                .next()
                .ok_or_else(|| RuntimeError::InvalidState("command cannot be empty".to_string()))?;
            let mut process = Command::new(executable);
            for arg in split {
                process.arg(arg);
            }
            process
        };

        if let Some(cwd) = cwd.as_deref() {
            proc.current_dir(cwd);
        }

        proc.kill_on_drop(true);
        proc.stdout(std::process::Stdio::piped());
        proc.stderr(std::process::Stdio::piped());

        let mut child = proc
            .spawn()
            .map_err(|error| RuntimeError::Io(format!("failed to spawn process: {error}")))?;

        let pid = child.id().map(i64::from);
        let started_at = now_ms();
        let record = ProcessRecord {
            id: process_id.clone(),
            session_id: request.caller_session_id.clone(),
            tool_call_id: request.tool_call_id,
            pid,
            command: json!({ "command": command }),
            cwd: cwd.clone(),
            status: "running".to_string(),
            exit_code: None,
            signal: None,
            stdout_path: Some(stdout_path.display().to_string()),
            stderr_path: Some(stderr_path.display().to_string()),
            started_at,
            ended_at: None,
            timeout_ms: Some(request.timeout_ms.unwrap_or(self.config.default_timeout_ms) as i64),
        };

        if let Err(error) = self.store.upsert_process(&record) {
            Self::teardown_untracked_child(&mut child).await;
            return Err(error);
        }

        let managed = Arc::new(ManagedProcess::new(record, Some(child)));
        {
            let mut processes = self.processes.write().await;
            processes.insert(process_id.clone(), Arc::clone(&managed));
        }

        self.append_process_event(
            process_id.as_str(),
            request.caller_session_id,
            "process.started",
            RuntimeEventCriticality::Critical,
            json!({
                "process_id": process_id,
                "pid": pid,
                "cwd": cwd,
            }),
        )
        .await;

        let manager = Arc::new(self.clone());
        tokio::spawn(async move {
            manager.run_lifecycle(managed, spawn_permit).await;
        });

        let process = self.process_from_id(process_id.as_str()).await?;
        Ok(Self::detail_from_process(process.as_ref()).await)
    }

    async fn list_processes(
        &self,
        request: ProcessListRequest,
    ) -> Result<Vec<ProcessSummary>, RuntimeError> {
        self.list_process_entries(
            request.caller_session_id.as_deref(),
            request.include_completed,
        )
        .await
    }

    async fn get_process(
        &self,
        request: ProcessGetRequest,
    ) -> Result<ProcessDetails, RuntimeError> {
        let process = self.process_from_id(request.process_id.as_str()).await?;
        self.ensure_ownership(process.as_ref(), request.caller_session_id.as_deref())
            .await?;
        Ok(Self::detail_from_process(process.as_ref()).await)
    }

    async fn read_process_logs(
        &self,
        request: ProcessLogReadRequest,
    ) -> Result<Vec<ProcessLogsChunk>, RuntimeError> {
        let process = self.process_from_id(request.process_id.as_str()).await?;
        self.ensure_ownership(process.as_ref(), request.caller_session_id.as_deref())
            .await?;

        let details = Self::detail_from_process(process.as_ref()).await;
        let mut streams = Vec::new();
        match request.stream.as_deref() {
            Some("stdout") => streams.push((
                "stdout",
                details.stdout_path.clone(),
                details.stdout_truncated,
            )),
            Some("stderr") => streams.push((
                "stderr",
                details.stderr_path.clone(),
                details.stderr_truncated,
            )),
            Some(other) => {
                return Err(RuntimeError::InvalidState(format!(
                    "unsupported stream {}",
                    other
                )))
            }
            None => {
                streams.push((
                    "stdout",
                    details.stdout_path.clone(),
                    details.stdout_truncated,
                ));
                streams.push((
                    "stderr",
                    details.stderr_path.clone(),
                    details.stderr_truncated,
                ));
            }
        }

        let mut chunks = Vec::new();
        for (stream, path, stream_truncated) in streams {
            let Some(path) = path else {
                continue;
            };
            let content = std::fs::read_to_string(Path::new(path.as_str())).unwrap_or_default();
            let lines = content.lines().collect::<Vec<_>>();
            let head = request.head_lines.unwrap_or(0);
            let tail = request.tail_lines.unwrap_or(80);

            let mut out = String::new();
            let mut truncated = false;

            if head > 0 {
                for line in lines.iter().take(head) {
                    out.push_str(line);
                    out.push('\n');
                }
            }

            let tail_start = lines.len().saturating_sub(tail);
            if head > 0 && tail_start > head {
                truncated = true;
                out.push_str("...\n");
            }

            for line in lines.iter().skip(tail_start) {
                out.push_str(line);
                out.push('\n');
            }

            if request.max_bytes.is_some() {
                let max_bytes = request.max_bytes.unwrap_or(64 * 1024);
                if out.as_bytes().len() > max_bytes {
                    let truncated_bytes = &out.as_bytes()[out.as_bytes().len() - max_bytes..];
                    out = String::from_utf8_lossy(truncated_bytes).to_string();
                    truncated = true;
                }
            }

            chunks.push(ProcessLogsChunk {
                process_id: details.process.process_id.clone(),
                stream: stream.to_string(),
                bytes: out.as_bytes().len(),
                content: out,
                head_lines: head,
                tail_lines: tail,
                truncated: truncated || stream_truncated,
            });
        }

        Ok(chunks)
    }

    async fn kill_process(
        &self,
        request: ProcessKillRequest,
    ) -> Result<ProcessDetails, RuntimeError> {
        let process = self.process_from_id(request.process_id.as_str()).await?;
        self.ensure_ownership(process.as_ref(), request.caller_session_id.as_deref())
            .await?;

        {
            let mut kill_requested = process.kill_requested.lock().await;
            *kill_requested = true;
        }

        let mut killed = false;
        {
            let mut child = process.child.lock().await;
            if let Some(child) = child.as_mut() {
                let _ = child.start_kill();
                killed = true;
            }
        }

        if killed {
            let record = process.record.lock().await;
            self.append_process_event(
                record.id.as_str(),
                record.session_id.clone(),
                "process.kill_requested",
                RuntimeEventCriticality::Critical,
                json!({
                    "reason": request.reason.unwrap_or_else(|| "requested".to_string()),
                    "process_id": record.id,
                }),
            )
            .await;
        }

        Ok(Self::detail_from_process(process.as_ref()).await)
    }

    async fn replay_events(
        &self,
        process_id: String,
        caller_session_id: Option<String>,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<runtime_core::RuntimeEventRecord>, RuntimeError> {
        let process = self.process_from_id(process_id.as_str()).await?;
        self.ensure_ownership(process.as_ref(), caller_session_id.as_deref())
            .await?;
        self.store.list_runtime_events(
            Some((RuntimeEventScope::Process, process_id.as_str())),
            after_seq,
            limit.max(1),
        )
    }

    fn subscribe_events(&self) -> broadcast::Receiver<runtime_core::RuntimeEventRecord> {
        self.event_tx.subscribe()
    }
}

impl RuntimeProcessManager {
    async fn teardown_untracked_child(child: &mut tokio::process::Child) {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
    }
}

impl Clone for RuntimeProcessManager {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            semaphore: Arc::clone(&self.semaphore),
            next_process_id: Arc::clone(&self.next_process_id),
            next_event_id: Arc::clone(&self.next_event_id),
            processes: Arc::clone(&self.processes),
            event_tx: self.event_tx.clone(),
        }
    }
}

pub struct RuntimeToolGateway {
    process_manager: Arc<RuntimeProcessManager>,
}

impl RuntimeToolGateway {
    pub fn new(process_manager: Arc<RuntimeProcessManager>) -> Self {
        Self { process_manager }
    }

    async fn invoke_process_tool(&self, request: ToolInvokeRequest) -> Value {
        let tool_name = request.tool_name.trim();
        let args = match request.args {
            Value::Object(map) => map,
            _ => {
                return json!({
                    "ok": false,
                    "error": {
                        "code": "bad_request",
                        "message": "tool args must be an object"
                    }
                });
            }
        };

        let result = match tool_name {
            GG_PROCESS_RUN => {
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
                let timeout_ms = args.get("timeout_ms").and_then(Value::as_u64);
                self.process_manager
                    .run_process(ProcessRunRequest {
                        caller_session_id: Some(request.caller_session_id.clone()),
                        tool_call_id: request.invocation_id.clone(),
                        command,
                        cwd,
                        timeout_ms,
                    })
                    .await
                    .map(|value| json!(value))
            }
            GG_PROCESS_STATUS => {
                if let Some(process_id) = args.get("process_id").and_then(Value::as_str) {
                    self.process_manager
                        .get_process(ProcessGetRequest {
                            process_id: process_id.to_string(),
                            caller_session_id: Some(request.caller_session_id.clone()),
                        })
                        .await
                        .map(|value| json!(value))
                } else if let Some(pid) = args.get("pid").and_then(Value::as_i64) {
                    let process = self.process_manager.process_from_pid(pid).await;
                    match process {
                        Ok(process) => {
                            let record = process.record.lock().await;
                            self.process_manager
                                .get_process(ProcessGetRequest {
                                    process_id: record.id.clone(),
                                    caller_session_id: Some(request.caller_session_id.clone()),
                                })
                                .await
                                .map(|value| json!(value))
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    self.process_manager
                        .list_processes(ProcessListRequest {
                            caller_session_id: Some(request.caller_session_id.clone()),
                            include_completed: false,
                        })
                        .await
                        .map(|rows| json!({ "running": rows }))
                }
            }
            GG_PROCESS_LOGS => {
                let process_id = args
                    .get("process_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let stream = args
                    .get("stream")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let head_lines = args
                    .get("head_lines")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize);
                let tail_lines = args
                    .get("tail_lines")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize);
                self.process_manager
                    .read_process_logs(ProcessLogReadRequest {
                        process_id,
                        caller_session_id: Some(request.caller_session_id.clone()),
                        stream,
                        head_lines,
                        tail_lines,
                        max_bytes: None,
                    })
                    .await
                    .map(|rows| json!({ "logs": rows }))
            }
            GG_PROCESS_KILL => {
                let process_id = args
                    .get("process_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                self.process_manager
                    .kill_process(ProcessKillRequest {
                        process_id,
                        caller_session_id: Some(request.caller_session_id),
                        reason: Some("gg_process_kill".to_string()),
                    })
                    .await
                    .map(|value| json!(value))
            }
            _ => Err(RuntimeError::Unsupported(format!(
                "Unsupported gg_process tool: {tool_name}"
            ))),
        };

        match result {
            Ok(result) => json!({ "ok": true, "result": result }),
            Err(error) => json!({
                "ok": false,
                "error": {
                    "code": "tool_failed",
                    "message": error.to_string(),
                }
            }),
        }
    }
}

#[async_trait]
impl ToolGateway for RuntimeToolGateway {
    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        self.process_manager.healthcheck().await
    }

    async fn invoke_tool(&self, request: ToolInvokeRequest) -> Result<Value, RuntimeError> {
        let caller_session_id = request.caller_session_id.trim();
        if caller_session_id.is_empty() {
            return Err(RuntimeError::InvalidState(
                "caller_session_id is required".to_string(),
            ));
        }

        if let Some(namespace) = request.namespace.as_deref() {
            if !namespace_matches_tool(namespace, request.tool_name.as_str()) {
                return Err(RuntimeError::InvalidState(
                    "namespace does not match tool_name".to_string(),
                ));
            }
        }

        if request.tool_name.starts_with("gg_process_") {
            return Ok(self.invoke_process_tool(request).await);
        }

        Ok(json!({
            "ok": false,
            "error": {
                "code": "bad_request",
                "message": format!("Unsupported tool name: {}", request.tool_name),
            }
        }))
    }

    async fn capabilities(&self) -> Result<Value, RuntimeError> {
        Ok(json!({
            "ok": true,
            "result": {
                "ggProcessEnabled": self.process_manager.config.enabled,
                "supportedNamespaces": ["gg_process"],
                "tools": [GG_PROCESS_RUN, GG_PROCESS_STATUS, GG_PROCESS_LOGS, GG_PROCESS_KILL],
            }
        }))
    }
}

fn namespace_matches_tool(namespace: &str, tool_name: &str) -> bool {
    match namespace.trim() {
        "gg_process" => tool_name.starts_with("gg_process_"),
        _ => false,
    }
}

#[derive(Debug)]
pub struct StubTeamCommsService {
    config: TeamCommsConfig,
}

#[derive(Debug)]
pub struct StubWorktreeService {
    config: WorktreeServiceConfig,
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

fn parse_process_sequence(process_id: &str) -> Option<u64> {
    process_id
        .strip_prefix("proc_")
        .and_then(|value| value.parse::<u64>().ok())
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_millis().min(i64::MAX as u128)) as i64
}

#[cfg(unix)]
fn exit_status_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_status_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_core::{
        ApprovalRecord, ManagedWorktreeClaimRecord, ManagedWorktreeRecord, ProcessListRequest,
        SessionRecord, TeamDeliveryRecord, TeamMemberRecord, TeamMessageRecord, TeamRecord,
        TurnRecord,
    };
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FailingProcessUpsertStore {
        last_pid: Mutex<Option<i64>>,
        upsert_process_calls: AtomicU64,
    }

    #[async_trait]
    impl RuntimeStore for FailingProcessUpsertStore {
        async fn initialize(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        async fn healthcheck(&self) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn append_runtime_event(
            &self,
            _event: &NewRuntimeEvent,
        ) -> Result<runtime_core::RuntimeEventRecord, RuntimeError> {
            Err(RuntimeError::Io(
                "event append should not be called in this test".to_string(),
            ))
        }

        fn list_runtime_events(
            &self,
            _scope: Option<(RuntimeEventScope, &str)>,
            _after_seq: Option<i64>,
            _limit: usize,
        ) -> Result<Vec<runtime_core::RuntimeEventRecord>, RuntimeError> {
            Ok(Vec::new())
        }

        fn upsert_session(&self, _record: &SessionRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_turn(&self, _record: &TurnRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_approval(&self, _record: &ApprovalRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_team(&self, _record: &TeamRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_team_member(&self, _record: &TeamMemberRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_team_message(&self, _record: &TeamMessageRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_team_delivery(&self, _record: &TeamDeliveryRecord) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_managed_worktree(
            &self,
            _record: &ManagedWorktreeRecord,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_managed_worktree_claim(
            &self,
            _record: &ManagedWorktreeClaimRecord,
        ) -> Result<(), RuntimeError> {
            Ok(())
        }

        fn upsert_process(&self, record: &ProcessRecord) -> Result<(), RuntimeError> {
            self.upsert_process_calls
                .fetch_add(1, AtomicOrdering::Relaxed);
            *self.last_pid.lock().expect("last pid mutex poisoned") = record.pid;
            Err(RuntimeError::Io(
                "forced upsert_process failure".to_string(),
            ))
        }

        fn hydrate_runtime_state(
            &self,
        ) -> Result<runtime_core::RuntimeHydratedState, RuntimeError> {
            Ok(runtime_core::RuntimeHydratedState::default())
        }
    }

    #[tokio::test]
    async fn spawn_failure_after_launch_tears_down_child_and_leaves_no_ghost_process() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(FailingProcessUpsertStore::default());
        let manager = RuntimeProcessManager::new(
            store.clone(),
            ProcessManagerConfig {
                enabled: true,
                max_concurrent: 1,
                default_timeout_ms: 60_000,
                max_output_bytes_per_process: 1_000_000,
                allow_shell: true,
                completed_retention_ms: 60_000,
                output_event_sample_bytes: 1024,
                log_dir: temp_dir.path().join("logs"),
            },
        )
        .await
        .expect("build process manager");

        let result = manager
            .run_process(ProcessRunRequest {
                caller_session_id: Some("sess_test".to_string()),
                tool_call_id: None,
                command: "sleep 5".to_string(),
                cwd: None,
                timeout_ms: None,
            })
            .await;
        assert!(matches!(result, Err(RuntimeError::Io(_))));

        // The process start failed after spawn. The fix must fail closed:
        // no retained managed process entry and the pre-handoff child torn down.
        let rows = manager
            .list_processes(ProcessListRequest {
                caller_session_id: Some("sess_test".to_string()),
                include_completed: true,
            })
            .await
            .expect("list processes");
        assert!(
            rows.is_empty(),
            "expected no retained process entries after failed start"
        );
        assert_eq!(
            store.upsert_process_calls.load(AtomicOrdering::Relaxed),
            1,
            "expected one failing upsert_process call"
        );

        #[cfg(unix)]
        {
            let pid = *store.last_pid.lock().expect("last pid mutex poisoned");
            if let Some(pid) = pid {
                let mut still_alive = true;
                for _ in 0..40 {
                    let status = std::process::Command::new("sh")
                        .arg("-lc")
                        .arg(format!("kill -0 {pid} >/dev/null 2>&1"))
                        .status()
                        .expect("kill -0 status");
                    if !status.success() {
                        still_alive = false;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                assert!(
                    !still_alive,
                    "spawned child pid {pid} remained alive after failed pre-handoff bootstrap"
                );
            }
        }
    }
}
