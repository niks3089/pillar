use std::time::Duration;

use pillar_shared::proto::{ExecuteScript, ScriptResult};
use tokio::io::AsyncBufReadExt;

const DEFAULT_TIMEOUT_SECS: u64 = 3600;
const SCRIPTS_DIR: &str = "/tmp/pillar-scripts";

pub struct ScriptExecutor;

impl ScriptExecutor {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(&self, cmd: ExecuteScript, node_id: &str) -> ScriptResult {
        let script_id = cmd.script_id.clone();
        let timeout_secs = if cmd.timeout_secs == 0 {
            DEFAULT_TIMEOUT_SECS
        } else {
            cmd.timeout_secs as u64
        };

        tracing::info!(
            script_id = %script_id,
            description = %cmd.description,
            timeout_secs,
            "executing script"
        );

        // Create scripts directory
        if let Err(e) = tokio::fs::create_dir_all(SCRIPTS_DIR).await {
            return ScriptResult {
                node_id: node_id.to_string(),
                script_id,
                exit_code: -1,
                timed_out: false,
                error: format!("failed to create scripts dir: {e}"),
                stdout: String::new(),
                stderr: String::new(),
            };
        }

        // Write script to temp file
        let script_path = format!("{SCRIPTS_DIR}/{}.sh", cmd.script_id);
        if let Err(e) = tokio::fs::write(&script_path, &cmd.script).await {
            return ScriptResult {
                node_id: node_id.to_string(),
                script_id,
                exit_code: -1,
                timed_out: false,
                error: format!("failed to write script file: {e}"),
                stdout: String::new(),
                stderr: String::new(),
            };
        }

        // Spawn process with process group for clean kill
        let child = unsafe {
            tokio::process::Command::new("sudo")
                .args(["bash", &script_path])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .pre_exec(|| {
                    libc::setsid();
                    Ok(())
                })
                .spawn()
        };

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = tokio::fs::remove_file(&script_path).await;
                return ScriptResult {
                    node_id: node_id.to_string(),
                    script_id,
                    exit_code: -1,
                    timed_out: false,
                    error: format!("failed to spawn process: {e}"),
                    stdout: String::new(),
                    stderr: String::new(),
                };
            }
        };

        // Read stdout and stderr concurrently, logging each line
        let stdout_pipe = child.stdout.take().unwrap();
        let stderr_pipe = child.stderr.take().unwrap();

        let sid = script_id.clone();
        let stdout_handle = tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stdout_pipe).lines();
            let mut output = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::info!(script_id = %sid, "[stdout] {}", line);
                output.push_str(&line);
                output.push('\n');
            }
            output
        });

        let sid = script_id.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr_pipe).lines();
            let mut output = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(script_id = %sid, "[stderr] {}", line);
                output.push_str(&line);
                output.push('\n');
            }
            output
        });

        // Wait with timeout
        let timeout = Duration::from_secs(timeout_secs);
        let result = tokio::time::timeout(timeout, child.wait()).await;

        let (exit_code, timed_out, error) = match result {
            Ok(Ok(status)) => (status.code().unwrap_or(-1), false, String::new()),
            Ok(Err(e)) => (-1, false, format!("wait error: {e}")),
            Err(_) => {
                // Timeout — kill the process group
                tracing::error!(script_id = %script_id, "script timed out after {timeout_secs}s, killing");
                let _ = child.kill().await;
                (-1, true, format!("timed out after {timeout_secs}s"))
            }
        };

        // Collect stdout/stderr
        let stdout = stdout_handle.await.unwrap_or_default();
        let stderr = stderr_handle.await.unwrap_or_default();

        // Clean up temp file
        let _ = tokio::fs::remove_file(&script_path).await;

        ScriptResult {
            node_id: node_id.to_string(),
            script_id,
            exit_code,
            timed_out,
            error,
            stdout,
            stderr,
        }
    }
}
