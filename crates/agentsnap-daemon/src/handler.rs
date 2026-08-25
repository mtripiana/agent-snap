use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use agentsnap_core::{
    execute_in_sandbox, preflight_hardened_report, ExecutionOptions, IsolationMode, NetworkMode,
    ResourceLimits, SecurityPolicy, ShellMode, SnapshotConfig, SnapshotSession,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug)]
pub struct DaemonState {
    scratch_root: PathBuf,
    sessions: HashMap<String, SnapshotSession>,
}

const MAX_SESSIONS: usize = 64;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 60 * 60 * 1000;

impl DaemonState {
    pub fn new(scratch_root: impl Into<PathBuf>) -> Self {
        Self {
            scratch_root: scratch_root.into(),
            sessions: HashMap::new(),
        }
    }

    pub fn handle_request_json(&mut self, input: &[u8]) -> Value {
        let request: RpcRequest = match serde_json::from_slice(input) {
            Ok(value) => value,
            Err(err) => {
                return rpc_error(None, -32700, &format!("invalid json: {err}"));
            }
        };

        if request.jsonrpc.as_deref().is_some_and(|version| version != "2.0") {
            return rpc_error(request.id, -32600, "unsupported JSON-RPC version");
        }

        match request.method.as_str() {
            "agentsnap.create_snapshot" => self.handle_create_snapshot(request.id, request.params),
            "agentsnap.execute" => self.handle_execute(request.id, request.params),
            "agentsnap.rollback" => self.handle_rollback(request.id, request.params),
            "agentsnap.doctor" => self.handle_doctor(request.id),
            _ => rpc_error(request.id, -32601, "method not found"),
        }
    }

    fn handle_create_snapshot(&mut self, id: Option<Value>, params: Option<Value>) -> Value {
        let params = match decode_params::<CreateSnapshotParams>(params) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, -32602, &message),
        };

        let started = Instant::now();
        if self.sessions.len() >= MAX_SESSIONS {
            return rpc_error(id, -32004, "maximum active snapshot count reached");
        }

        let config = SnapshotConfig {
            base_dir: PathBuf::from(params.workspace),
            scratch_dir: self.scratch_root.clone(),
            read_only: params.read_only.unwrap_or(false),
        };

        match SnapshotSession::create(&config) {
            Ok(session) => {
                let snapshot_id = session.id.clone();
                self.sessions.insert(snapshot_id.clone(), session);
                rpc_result(
                    id,
                    json!({
                        "snapshot_id": snapshot_id,
                        "latency_ms": started.elapsed().as_secs_f64() * 1000.0,
                    }),
                )
            }
            Err(err) => rpc_error(id, -32000, &format!("snapshot creation failed: {err}")),
        }
    }

    fn handle_execute(&mut self, id: Option<Value>, params: Option<Value>) -> Value {
        let params = match decode_params::<ExecuteParams>(params) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, -32602, &message),
        };

        let session = match self.sessions.get(&params.snapshot_id) {
            Some(session) => session,
            None => return rpc_error(id, -32001, "unknown snapshot_id"),
        };

        let policy = match parse_policy(params.policy.clone()) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, -32602, &message),
        };

        let execution_shape = match parse_command_shape(&params, policy.shell_mode) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, -32602, &message),
        };

        let options = ExecutionOptions {
            command: execution_shape.command,
            args: execution_shape.args,
            env: params.env.unwrap_or_default(),
            timeout_ms: params.timeout_ms.unwrap_or(30_000),
            policy: policy.clone(),
            is_shell_command: execution_shape.is_shell_command,
            max_output_bytes: params
                .max_output_bytes
                .unwrap_or(DEFAULT_MAX_OUTPUT_BYTES),
        };

        if options.timeout_ms == 0 || options.timeout_ms > MAX_TIMEOUT_MS {
            return rpc_error(
                id,
                -32602,
                "timeout_ms must be between 1 and 3600000",
            );
        }
        if options.max_output_bytes == 0 || options.max_output_bytes > MAX_OUTPUT_BYTES {
            return rpc_error(
                id,
                -32602,
                "max_output_bytes must be between 1 and 67108864",
            );
        }

        match execute_in_sandbox(session, options) {
            Ok(result) => {
                let response = rpc_result(
                    id,
                    json!({
                        "exit_code": result.exit_code,
                        "stdout": result.stdout,
                        "stderr": result.stderr,
                        "duration_ms": result.duration_ms,
                        "effective_isolation": result.effective_isolation.as_str(),
                        "enforcement_report": {
                            "requested_isolation": result.enforcement_report.requested_isolation.as_str(),
                            "effective_isolation": result.enforcement_report.effective_isolation.as_str(),
                            "hardening_enforced": result.enforcement_report.hardening_enforced,
                            "notes": result.enforcement_report.notes,
                        }
                    }),
                );

                emit_audit_event(json!({
                    "event": "execution_completed",
                    "snapshot_id": params.snapshot_id,
                    "requested_isolation": policy.isolation_mode.as_str(),
                    "effective_isolation": result.effective_isolation.as_str(),
                    "network_mode": policy.network_mode.as_str(),
                    "shell_mode": policy.shell_mode.as_str(),
                    "seccomp_profile": policy.seccomp_profile,
                    "duration_ms": result.duration_ms,
                    "exit_code": result.exit_code,
                }));

                response
            }
            Err(err) => {
                emit_audit_event(json!({
                    "event": "execution_failed",
                    "snapshot_id": params.snapshot_id,
                    "requested_isolation": policy.isolation_mode.as_str(),
                    "network_mode": policy.network_mode.as_str(),
                    "shell_mode": policy.shell_mode.as_str(),
                    "seccomp_profile": policy.seccomp_profile,
                    "error": err.to_string(),
                }));

                rpc_error(id, -32002, &format!("execution failed: {err}"))
            }
        }
    }

    fn handle_rollback(&mut self, id: Option<Value>, params: Option<Value>) -> Value {
        let params = match decode_params::<RollbackParams>(params) {
            Ok(value) => value,
            Err(message) => return rpc_error(id, -32602, &message),
        };

        let session = match self.sessions.remove(&params.snapshot_id) {
            Some(session) => session,
            None => return rpc_error(id, -32001, "unknown snapshot_id"),
        };

        let started = Instant::now();
        match session.rollback() {
            Ok(()) => rpc_result(
                id,
                json!({
                    "status": "restored",
                    "duration_ms": started.elapsed().as_secs_f64() * 1000.0,
                }),
            ),
            Err(err) => rpc_error(id, -32003, &format!("rollback failed: {err}")),
        }
    }

    fn handle_doctor(&self, id: Option<Value>) -> Value {
        let report = preflight_hardened_report();
        rpc_result(
            id,
            json!({
                "hardened_supported": report.hardened_supported,
                "checks": report
                    .checks
                    .into_iter()
                    .map(|check| {
                        json!({
                            "name": check.name,
                            "ok": check.ok,
                            "detail": check.detail,
                        })
                    })
                    .collect::<Vec<_>>(),
            }),
        )
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct CreateSnapshotParams {
    workspace: String,
    read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ExecuteParams {
    snapshot_id: String,
    command: Option<String>,
    command_path: Option<String>,
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
    timeout_ms: Option<u64>,
    policy: Option<PolicyParams>,
    max_output_bytes: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct PolicyParams {
    isolation_mode: Option<String>,
    network_mode: Option<String>,
    shell_mode: Option<String>,
    resource_limits: Option<ResourceLimitsParams>,
    seccomp_profile: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ResourceLimitsParams {
    memory_limit_bytes: Option<u64>,
    cpu_quota_percent: Option<u64>,
    pids_max: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RollbackParams {
    snapshot_id: String,
}

#[derive(Debug)]
struct ParsedCommandShape {
    command: String,
    args: Vec<String>,
    is_shell_command: bool,
}

fn parse_command_shape(params: &ExecuteParams, shell_mode: ShellMode) -> std::result::Result<ParsedCommandShape, String> {
    if let Some(command_line) = &params.command {
        if shell_mode == ShellMode::Disallow {
            return Err("shell command string is disallowed by policy; use command_path + args".to_string());
        }

        return Ok(ParsedCommandShape {
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), command_line.clone()],
            is_shell_command: true,
        });
    }

    let command_path = params.command_path.clone().unwrap_or_default();
    if command_path.trim().is_empty() {
        return Err("either params.command or params.command_path is required".to_string());
    }

    if shell_mode == ShellMode::Disallow && is_shell_command_path(&command_path) {
        return Err("shell executables are disallowed by policy; use a non-shell command_path".to_string());
    }

    Ok(ParsedCommandShape {
        command: command_path,
        args: params.args.clone().unwrap_or_default(),
        is_shell_command: false,
    })
}

fn is_shell_command_path(command_path: &str) -> bool {
    let name = std::path::Path::new(command_path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    matches!(
        name.as_str(),
        "sh" | "bash" | "dash" | "zsh" | "fish" | "ksh" | "csh" | "tcsh" | "powershell" | "pwsh"
    )
}

fn parse_policy(input: Option<PolicyParams>) -> std::result::Result<SecurityPolicy, String> {
    let mut policy = SecurityPolicy::default();

    if let Some(input) = input {
        if let Some(mode) = input.isolation_mode {
            policy.isolation_mode = parse_isolation_mode(&mode)?;
        }

        if let Some(mode) = input.network_mode {
            policy.network_mode = parse_network_mode(&mode)?;
        }

        if let Some(mode) = input.shell_mode {
            policy.shell_mode = parse_shell_mode(&mode)?;
        }

        if let Some(profile) = input.seccomp_profile {
            policy.seccomp_profile = profile;
        }

        if let Some(limits) = input.resource_limits {
            let mut resolved = ResourceLimits::default();
            if let Some(memory_limit_bytes) = limits.memory_limit_bytes {
                resolved.memory_limit_bytes = memory_limit_bytes;
            }
            if let Some(cpu_quota_percent) = limits.cpu_quota_percent {
                resolved.cpu_quota_percent = cpu_quota_percent;
            }
            if let Some(pids_max) = limits.pids_max {
                resolved.pids_max = pids_max;
            }
            policy.resource_limits = resolved;
        }
    }

    if policy.isolation_mode == IsolationMode::Hardened {
        if policy.network_mode != NetworkMode::None {
            return Err("hardened mode requires policy.network_mode=none".to_string());
        }
        if policy.shell_mode != ShellMode::Disallow {
            return Err("hardened mode requires policy.shell_mode=disallow".to_string());
        }
    }

    if policy.resource_limits.memory_limit_bytes == 0
        || policy.resource_limits.cpu_quota_percent == 0
        || policy.resource_limits.pids_max == 0
    {
        return Err("resource limits must be greater than 0".to_string());
    }

    if !matches!(policy.seccomp_profile.as_str(), "default_script" | "default_build") {
        return Err(format!(
            "unsupported seccomp_profile: {}",
            policy.seccomp_profile
        ));
    }

    Ok(policy)
}

fn parse_isolation_mode(value: &str) -> std::result::Result<IsolationMode, String> {
    match value {
        "best_effort" => Ok(IsolationMode::BestEffort),
        "hardened" => Ok(IsolationMode::Hardened),
        _ => Err(format!("unsupported isolation_mode: {value}")),
    }
}

fn parse_network_mode(value: &str) -> std::result::Result<NetworkMode, String> {
    match value {
        "none" => Ok(NetworkMode::None),
        "restricted" => Ok(NetworkMode::Restricted),
        _ => Err(format!("unsupported network_mode: {value}")),
    }
}

fn parse_shell_mode(value: &str) -> std::result::Result<ShellMode, String> {
    match value {
        "allow" => Ok(ShellMode::Allow),
        "disallow" => Ok(ShellMode::Disallow),
        _ => Err(format!("unsupported shell_mode: {value}")),
    }
}

fn decode_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> std::result::Result<T, String> {
    let value = params.unwrap_or_else(|| json!({}));
    serde_json::from_value(value).map_err(|err| format!("invalid params: {err}"))
}

fn rpc_result(id: Option<Value>, result: Value) -> Value {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
    .into_value()
}

fn rpc_error(id: Option<Value>, code: i32, message: &str) -> Value {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code,
            message: message.to_string(),
        }),
    }
    .into_value()
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

impl RpcResponse {
    fn into_value(self) -> Value {
        serde_json::to_value(self).expect("response serialization")
    }
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

fn emit_audit_event(event: Value) {
    eprintln!("agentsnap_audit={event}");
}

pub fn default_scratch_root() -> &'static Path {
    Path::new("/tmp/agentsnap/scratchpads")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::DaemonState;

    #[test]
    fn create_execute_rollback_roundtrip() {
        let workspace = tempdir().expect("workspace");
        let scratch = tempdir().expect("scratch");

        fs::write(workspace.path().join("note.txt"), "hello").expect("seed");

        let mut state = DaemonState::new(scratch.path());

        let create = state.handle_request_json(
            json!({
                "id": 1,
                "method": "agentsnap.create_snapshot",
                "params": {"workspace": workspace.path().display().to_string()},
            })
            .to_string()
            .as_bytes(),
        );

        let snapshot_id = create["result"]["snapshot_id"]
            .as_str()
            .expect("snapshot id")
            .to_string();

        let exec = state.handle_request_json(
            json!({
                "id": 2,
                "method": "agentsnap.execute",
                "params": {
                    "snapshot_id": snapshot_id,
                    "command": "cat note.txt"
                }
            })
            .to_string()
            .as_bytes(),
        );

        assert_eq!(exec["result"]["exit_code"], 0);
        assert_eq!(exec["result"]["effective_isolation"], "best_effort");
        assert!(exec["result"]["stdout"].as_str().unwrap_or_default().contains("hello"));

        let rollback = state.handle_request_json(
            json!({
                "id": 3,
                "method": "agentsnap.rollback",
                "params": {"snapshot_id": create["result"]["snapshot_id"]},
            })
            .to_string()
            .as_bytes(),
        );

        assert_eq!(rollback["result"]["status"], "restored");
    }

    #[test]
    fn rollback_restores_after_rm_rf_src_simulation() {
        let workspace = tempdir().expect("workspace");
        let scratch = tempdir().expect("scratch");

        let src_dir = workspace.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(src_dir.join("main.txt"), "hello-src").expect("seed src file");

        let mut state = DaemonState::new(scratch.path());

        let create_first = state.handle_request_json(
            json!({
                "id": 10,
                "method": "agentsnap.create_snapshot",
                "params": {"workspace": workspace.path().display().to_string()},
            })
            .to_string()
            .as_bytes(),
        );

        let first_snapshot_id = create_first["result"]["snapshot_id"]
            .as_str()
            .expect("first snapshot id")
            .to_string();

        let delete_exec = state.handle_request_json(
            json!({
                "id": 11,
                "method": "agentsnap.execute",
                "params": {
                    "snapshot_id": first_snapshot_id,
                    "command": "rm -rf src"
                }
            })
            .to_string()
            .as_bytes(),
        );

        assert_eq!(delete_exec["result"]["exit_code"], 0);

        let rollback_first = state.handle_request_json(
            json!({
                "id": 12,
                "method": "agentsnap.rollback",
                "params": {"snapshot_id": create_first["result"]["snapshot_id"]},
            })
            .to_string()
            .as_bytes(),
        );

        assert_eq!(rollback_first["result"]["status"], "restored");

        let create_second = state.handle_request_json(
            json!({
                "id": 13,
                "method": "agentsnap.create_snapshot",
                "params": {"workspace": workspace.path().display().to_string()},
            })
            .to_string()
            .as_bytes(),
        );

        let second_snapshot_id = create_second["result"]["snapshot_id"]
            .as_str()
            .expect("second snapshot id")
            .to_string();

        let verify_exec = state.handle_request_json(
            json!({
                "id": 14,
                "method": "agentsnap.execute",
                "params": {
                    "snapshot_id": second_snapshot_id,
                    "command": "test -f src/main.txt && cat src/main.txt"
                }
            })
            .to_string()
            .as_bytes(),
        );

        assert_eq!(verify_exec["result"]["exit_code"], 0);
        assert!(verify_exec["result"]["stdout"].as_str().unwrap_or_default().contains("hello-src"));
    }

    #[test]
    fn returns_error_for_unknown_method() {
        let scratch = tempdir().expect("scratch");
        let mut state = DaemonState::new(scratch.path());

        let response = state.handle_request_json(
            json!({
                "id": 5,
                "method": "agentsnap.unknown",
                "params": {},
            })
            .to_string()
            .as_bytes(),
        );

        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn doctor_returns_preflight_checks() {
        let scratch = tempdir().expect("scratch");
        let mut state = DaemonState::new(scratch.path());

        let response = state.handle_request_json(
            json!({
                "id": 8,
                "method": "agentsnap.doctor",
                "params": {},
            })
            .to_string()
            .as_bytes(),
        );

        assert!(response["result"]["checks"].is_array());
    }
}
