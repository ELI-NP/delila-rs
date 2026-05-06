use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use super::config::ProcessConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessState {
    Stopped,
    Running,
    Restarting,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct ProcessStatus {
    pub name: String,
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub started_at: Option<DateTime<Utc>>,
    pub restart_count: u32,
    pub auto_restart: bool,
    pub command: String,
}

#[derive(Debug, Serialize)]
pub struct LogResponse {
    pub name: String,
    pub lines: Vec<String>,
    pub total_lines_captured: usize,
}

pub enum ProcessCommand {
    Start,
    Stop,
}

struct ProcessInfo {
    state: ProcessState,
    pid: Option<u32>,
    exit_code: Option<i32>,
    started_at: Option<DateTime<Utc>>,
    restart_count: u32,
    log_buffer: VecDeque<String>,
    total_lines: usize,
}

pub struct ProcessHandle {
    config: ProcessConfig,
    info: Arc<RwLock<ProcessInfo>>,
    cmd_tx: mpsc::Sender<ProcessCommand>,
}

pub struct ProcessManager {
    processes: HashMap<String, ProcessHandle>,
    log_buffer_lines: usize,
}

impl ProcessManager {
    pub fn new(configs: Vec<ProcessConfig>, log_buffer_lines: usize) -> Self {
        let mut processes = HashMap::new();

        for config in configs {
            let name = config.name.clone();
            let info = Arc::new(RwLock::new(ProcessInfo {
                state: ProcessState::Stopped,
                pid: None,
                exit_code: None,
                started_at: None,
                restart_count: 0,
                log_buffer: VecDeque::with_capacity(log_buffer_lines),
                total_lines: 0,
            }));

            let (cmd_tx, cmd_rx) = mpsc::channel(8);

            // Spawn monitor task for this process
            let monitor_info = Arc::clone(&info);
            let monitor_config = config.clone();
            let max_lines = log_buffer_lines;
            tokio::spawn(async move {
                process_monitor(monitor_config, monitor_info, cmd_rx, max_lines).await;
            });

            processes.insert(
                name,
                ProcessHandle {
                    config,
                    info,
                    cmd_tx,
                },
            );
        }

        Self {
            processes,
            log_buffer_lines,
        }
    }

    pub async fn start(&self, name: &str) -> Result<(), String> {
        let handle = self
            .processes
            .get(name)
            .ok_or_else(|| format!("Process '{}' not found", name))?;

        let info = handle.info.read().await;
        if info.state == ProcessState::Running {
            return Err(format!("Process '{}' is already running", name));
        }
        drop(info);

        handle
            .cmd_tx
            .send(ProcessCommand::Start)
            .await
            .map_err(|_| format!("Monitor task for '{}' is not running", name))
    }

    pub async fn stop(&self, name: &str) -> Result<(), String> {
        let handle = self
            .processes
            .get(name)
            .ok_or_else(|| format!("Process '{}' not found", name))?;

        let info = handle.info.read().await;
        if info.state != ProcessState::Running {
            return Err(format!("Process '{}' is not running", name));
        }
        drop(info);

        handle
            .cmd_tx
            .send(ProcessCommand::Stop)
            .await
            .map_err(|_| format!("Monitor task for '{}' is not running", name))
    }

    pub async fn restart(&self, name: &str) -> Result<(), String> {
        let handle = self
            .processes
            .get(name)
            .ok_or_else(|| format!("Process '{}' not found", name))?;

        // Send stop then start
        let _ = handle.cmd_tx.send(ProcessCommand::Stop).await;
        handle
            .cmd_tx
            .send(ProcessCommand::Start)
            .await
            .map_err(|_| format!("Monitor task for '{}' is not running", name))
    }

    pub async fn start_all(&self) {
        for (name, handle) in &self.processes {
            let info = handle.info.read().await;
            if info.state != ProcessState::Running {
                drop(info);
                if let Err(e) = handle.cmd_tx.send(ProcessCommand::Start).await {
                    warn!(name, error = %e, "Failed to send start command");
                }
            }
        }
    }

    pub async fn stop_all(&self) {
        for (name, handle) in &self.processes {
            let info = handle.info.read().await;
            if info.state == ProcessState::Running {
                drop(info);
                if let Err(e) = handle.cmd_tx.send(ProcessCommand::Stop).await {
                    warn!(name, error = %e, "Failed to send stop command");
                }
            }
        }
    }

    pub async fn get_status(&self, name: &str) -> Option<ProcessStatus> {
        let handle = self.processes.get(name)?;
        let info = handle.info.read().await;
        Some(ProcessStatus {
            name: name.to_string(),
            state: info.state,
            pid: info.pid,
            exit_code: info.exit_code,
            started_at: info.started_at,
            restart_count: info.restart_count,
            auto_restart: handle.config.auto_restart,
            command: format!("{} {}", handle.config.command, handle.config.args.join(" ")),
        })
    }

    pub async fn all_status(&self) -> Vec<ProcessStatus> {
        let mut statuses = Vec::with_capacity(self.processes.len());
        for (name, handle) in &self.processes {
            let info = handle.info.read().await;
            statuses.push(ProcessStatus {
                name: name.clone(),
                state: info.state,
                pid: info.pid,
                exit_code: info.exit_code,
                started_at: info.started_at,
                restart_count: info.restart_count,
                auto_restart: handle.config.auto_restart,
                command: format!("{} {}", handle.config.command, handle.config.args.join(" ")),
            });
        }
        statuses
    }

    pub async fn get_logs(&self, name: &str, tail: Option<usize>) -> Option<LogResponse> {
        let handle = self.processes.get(name)?;
        let info = handle.info.read().await;
        let total = info.total_lines;
        let lines: Vec<String> = match tail {
            Some(n) => info
                .log_buffer
                .iter()
                .rev()
                .take(n)
                .rev()
                .cloned()
                .collect(),
            None => info.log_buffer.iter().cloned().collect(),
        };
        Some(LogResponse {
            name: name.to_string(),
            lines,
            total_lines_captured: total,
        })
    }

    #[allow(dead_code)]
    pub fn log_buffer_lines(&self) -> usize {
        self.log_buffer_lines
    }
}

async fn spawn_child(
    config: &ProcessConfig,
    info: &Arc<RwLock<ProcessInfo>>,
    max_lines: usize,
) -> Option<tokio::process::Child> {
    let mut cmd = tokio::process::Command::new(&config.command);
    cmd.args(&config.args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .envs(&config.env);

    if let Some(ref dir) = config.working_dir {
        cmd.current_dir(dir);
    }

    match cmd.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            info!(name = config.name, pid = pid, "Process started");

            // Capture stdout
            if let Some(stdout) = child.stdout.take() {
                let info_clone = Arc::clone(info);
                tokio::spawn(async move {
                    capture_output(stdout, info_clone, max_lines).await;
                });
            }

            // Capture stderr
            if let Some(stderr) = child.stderr.take() {
                let info_clone = Arc::clone(info);
                tokio::spawn(async move {
                    capture_output(stderr, info_clone, max_lines).await;
                });
            }

            let mut w = info.write().await;
            w.state = ProcessState::Running;
            w.pid = pid;
            w.exit_code = None;
            w.started_at = Some(Utc::now());

            Some(child)
        }
        Err(e) => {
            error!(name = config.name, error = %e, "Failed to spawn process");
            let mut w = info.write().await;
            w.state = ProcessState::Failed;
            None
        }
    }
}

async fn capture_output<R: tokio::io::AsyncRead + Unpin>(
    output: R,
    info: Arc<RwLock<ProcessInfo>>,
    max_lines: usize,
) {
    let reader = tokio::io::BufReader::new(output);
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut w = info.write().await;
        if w.log_buffer.len() >= max_lines {
            w.log_buffer.pop_front();
        }
        w.log_buffer.push_back(line);
        w.total_lines += 1;
    }
}

async fn process_monitor(
    config: ProcessConfig,
    info: Arc<RwLock<ProcessInfo>>,
    mut cmd_rx: mpsc::Receiver<ProcessCommand>,
    max_lines: usize,
) {
    let mut child: Option<tokio::process::Child> = None;
    let mut restart_at: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ProcessCommand::Start) => {
                        if child.is_some() {
                            continue;
                        }
                        restart_at = None;
                        child = spawn_child(&config, &info, max_lines).await;
                    }
                    Some(ProcessCommand::Stop) => {
                        // Cancel any pending restart
                        restart_at = None;
                        if let Some(ref mut c) = child {
                            info!(name = config.name, "Stopping process");
                            if let Err(e) = c.kill().await {
                                warn!(name = config.name, error = %e, "Failed to kill process");
                            }
                            let _ = c.wait().await;
                            child = None;
                        }
                        let mut w = info.write().await;
                        w.state = ProcessState::Stopped;
                        w.pid = None;
                    }
                    None => {
                        break;
                    }
                }
            }
            _ = async {
                tokio::time::sleep_until(
                    restart_at.expect("guarded by `if restart_at.is_some()` below"),
                )
                .await
            }, if restart_at.is_some() => {
                // Restart timer fired
                restart_at = None;
                child = spawn_child(&config, &info, max_lines).await;
            }
            status = async {
                match child.as_mut() {
                    Some(c) => c.wait().await,
                    None => std::future::pending().await,
                }
            } => {
                // Child exited on its own
                child = None;
                let exit_code = status.ok().and_then(|s| s.code());
                info!(
                    name = config.name,
                    exit_code = ?exit_code,
                    "Process exited"
                );

                if config.auto_restart {
                    let mut w = info.write().await;
                    w.state = ProcessState::Restarting;
                    w.pid = None;
                    w.exit_code = exit_code;
                    w.restart_count += 1;
                    drop(w);

                    info!(
                        name = config.name,
                        delay_secs = config.restart_delay_secs,
                        "Auto-restarting process"
                    );
                    restart_at = Some(
                        tokio::time::Instant::now()
                            + std::time::Duration::from_secs(config.restart_delay_secs),
                    );
                } else {
                    let mut w = info.write().await;
                    w.state = ProcessState::Failed;
                    w.pid = None;
                    w.exit_code = exit_code;
                }
            }
        }
    }
}
