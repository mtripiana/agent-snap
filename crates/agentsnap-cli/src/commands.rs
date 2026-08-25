use std::env;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use agentsnap_core::preflight_hardened_report;
use serde_json::{json, Value};

pub fn run(args: Vec<String>) -> Result<(), CliError> {
    if args.is_empty() {
        return Err(CliError::Usage(usage_text()));
    }

    match args[0].as_str() {
        "init" => init_daemon(args.get(1..).unwrap_or(&[])),
        "exec" => exec_command(args.get(1..).unwrap_or(&[])),
        "snap" => snap_command(args.get(1..).unwrap_or(&[])),
        "rollback" => rollback_command(args.get(1..).unwrap_or(&[])),
        "doctor" => doctor_command(args.get(1..).unwrap_or(&[])),
        _ => Err(CliError::Usage(usage_text())),
    }
}

fn init_daemon(_args: &[String]) -> Result<(), CliError> {
    let socket = socket_path();
    if socket.exists() {
        println!("daemon socket already exists at {}", socket.display());
        return Ok(());
    }

    let daemon_bin = env::var("AGENTSNAP_DAEMON_BIN").unwrap_or_else(|_| "agentsnap-daemon".to_string());
    let mut command = Command::new(daemon_bin);
    command
        .arg("--socket")
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let _child = command.spawn().map_err(|err| {
        CliError::Runtime(format!(
            "failed to start daemon. set AGENTSNAP_DAEMON_BIN if needed: {err}"
        ))
    })?;

    for _ in 0..100 {
        if UnixStream::connect(&socket).is_ok() {
            println!("daemon started, socket: {}", socket.display());
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    Err(CliError::Runtime(format!(
        "daemon did not become ready at {}",
        socket.display()
    )))
}

fn exec_command(args: &[String]) -> Result<(), CliError> {
    let workspace = take_flag_value(args, "--workspace")
        .ok_or_else(|| CliError::Usage("missing --workspace".to_string()))?;

    let isolation = take_flag_value(args, "--isolation").unwrap_or_else(|| "best_effort".to_string());
    let no_shell = has_flag(args, "--no-shell");

    let policy = json!({
        "isolation_mode": isolation,
        "network_mode": if take_flag_value(args, "--network").unwrap_or_else(|| "restricted".to_string()) == "none" { "none" } else { "restricted" },
        "shell_mode": if no_shell { "disallow" } else { "allow" },
        "seccomp_profile": take_flag_value(args, "--seccomp-profile").unwrap_or_else(|| "default_script".to_string()),
        "resource_limits": {
            "memory_limit_bytes": parse_memory_limit(args)?,
            "cpu_quota_percent": parse_cpu_limit(args)?,
            "pids_max": parse_pids_limit(args)?,
        }
    });

    let command_payload = if let Some(shell_command) = take_flag_value(args, "--command") {
        json!({"command": shell_command})
    } else {
        let command_path = take_flag_value(args, "--command-path")
            .ok_or_else(|| CliError::Usage("missing --command or --command-path".to_string()))?;
        let command_args = collect_repeated_values(args, "--arg");
        json!({"command_path": command_path, "args": command_args})
    };

    let create = rpc_call(
        "agentsnap.create_snapshot",
        json!({"workspace": workspace}),
    )?;

    let snapshot_id = create
        .get("result")
        .and_then(|result| result.get("snapshot_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::Runtime("daemon returned no snapshot_id".to_string()))?
        .to_string();

    let mut execute_params = json!({
        "snapshot_id": snapshot_id,
        "timeout_ms": take_flag_value(args, "--timeout-ms").and_then(|v| v.parse::<u64>().ok()).unwrap_or(30_000),
        "policy": policy,
    });

    if let Some(object) = execute_params.as_object_mut() {
        if let Some(command_object) = command_payload.as_object() {
            for (key, value) in command_object {
                object.insert(key.clone(), value.clone());
            }
        }
    }

    let exec_result = match rpc_call("agentsnap.execute", execute_params) {
        Ok(response) => response,
        Err(err) => {
            let _ = rpc_call(
                "agentsnap.rollback",
                json!({"snapshot_id": snapshot_id}),
            );
            return Err(err);
        }
    };

    if let Some(stdout) = exec_result
        .get("result")
        .and_then(|value| value.get("stdout"))
        .and_then(Value::as_str)
    {
        if !stdout.is_empty() {
            print!("{stdout}");
        }
    }

    if let Some(stderr) = exec_result
        .get("result")
        .and_then(|value| value.get("stderr"))
        .and_then(Value::as_str)
    {
        if !stderr.is_empty() {
            eprint!("{stderr}");
        }
    }

    if let Some(report) = exec_result
        .get("result")
        .and_then(|value| value.get("enforcement_report"))
    {
        eprintln!("enforcement_report={report}");
    }

    let exit_code = exec_result
        .get("result")
        .and_then(|value| value.get("exit_code"))
        .and_then(Value::as_i64)
        .unwrap_or(-1) as i32;

    let rollback_result = rpc_call(
        "agentsnap.rollback",
        json!({"snapshot_id": snapshot_id}),
    );

    if let Err(err) = rollback_result {
        return Err(CliError::Runtime(format!(
            "command completed but snapshot cleanup failed: {err}"
        )));
    }

    if exit_code != 0 {
        return Err(CliError::Runtime(format!("command exited with code {exit_code}")));
    }

    Ok(())
}

fn snap_command(args: &[String]) -> Result<(), CliError> {
    if args.first().map(String::as_str) != Some("create") {
        return Err(CliError::Usage("usage: agentsnap snap create --workspace <path>".to_string()));
    }

    let workspace = take_flag_value(args, "--workspace")
        .ok_or_else(|| CliError::Usage("missing --workspace".to_string()))?;

    let response = rpc_call(
        "agentsnap.create_snapshot",
        json!({"workspace": workspace}),
    )?;

    let snapshot_id = response
        .get("result")
        .and_then(|value| value.get("snapshot_id"))
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::Runtime("daemon returned no snapshot_id".to_string()))?;

    println!("{snapshot_id}");
    Ok(())
}

fn rollback_command(args: &[String]) -> Result<(), CliError> {
    let name = take_flag_value(args, "--name")
        .or_else(|| take_flag_value(args, "--snapshot-id"))
        .ok_or_else(|| CliError::Usage("missing --name or --snapshot-id".to_string()))?;

    let response = rpc_call("agentsnap.rollback", json!({"snapshot_id": name}))?;

    if response.get("error").is_some() {
        return Err(CliError::Runtime(format!("rollback failed: {response}")));
    }

    println!("restored");
    Ok(())
}

fn doctor_command(args: &[String]) -> Result<(), CliError> {
    let local_only = has_flag(args, "--local-only");

    if local_only {
        let report = preflight_hardened_report();
        print_local_doctor_report(&report);
        return Ok(());
    }

    match rpc_call("agentsnap.doctor", json!({})) {
        Ok(response) => {
            println!("{}", serde_json::to_string_pretty(&response).map_err(|err| CliError::Runtime(err.to_string()))?);
            Ok(())
        }
        Err(_) => {
            let report = preflight_hardened_report();
            print_local_doctor_report(&report);
            Ok(())
        }
    }
}

fn print_local_doctor_report(report: &agentsnap_core::security::SecurityPreflightReport) {
    println!(
        "{{\"hardened_supported\":{},\"checks\":[{}]}}",
        report.hardened_supported,
        report
            .checks
            .iter()
            .map(|check| {
                format!(
                    "{{\"name\":\"{}\",\"ok\":{},\"detail\":\"{}\"}}",
                    escape_json(&check.name),
                    check.ok,
                    escape_json(&check.detail)
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    );
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn rpc_call(method: &str, params: Value) -> Result<Value, CliError> {
    let mut stream = UnixStream::connect(socket_path())
        .map_err(|err| CliError::Runtime(format!("failed to connect daemon socket: {err}")))?;

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    })
    .to_string();

    stream
        .write_all(payload.as_bytes())
        .map_err(|err| CliError::Runtime(format!("failed to write request: {err}")))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|err| CliError::Runtime(format!("failed to finalize request: {err}")))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| CliError::Runtime(format!("failed to read response: {err}")))?;

    let value: Value = serde_json::from_str(response.trim())
        .map_err(|err| CliError::Runtime(format!("invalid daemon response: {err}")))?;

    if let Some(err) = value.get("error") {
        return Err(CliError::Runtime(format!("daemon error: {err}")));
    }

    Ok(value)
}

fn socket_path() -> PathBuf {
    env::var("AGENTSNAP_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/agentsnap.sock"))
}

fn take_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn collect_repeated_values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            if let Some(value) = args.get(index + 1) {
                values.push(value.clone());
            }
            index += 1;
        }
        index += 1;
    }
    values
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn parse_memory_limit(args: &[String]) -> Result<u64, CliError> {
    if let Some(value) = take_flag_value(args, "--memory-mb") {
        let parsed = value
            .parse::<u64>()
            .map_err(|_| CliError::Usage("--memory-mb must be an integer".to_string()))?;
        return Ok(parsed.saturating_mul(1024 * 1024));
    }
    Ok(512 * 1024 * 1024)
}

fn parse_cpu_limit(args: &[String]) -> Result<u64, CliError> {
    if let Some(value) = take_flag_value(args, "--cpu-quota") {
        return value
            .parse::<u64>()
            .map_err(|_| CliError::Usage("--cpu-quota must be an integer".to_string()));
    }
    Ok(100)
}

fn parse_pids_limit(args: &[String]) -> Result<u64, CliError> {
    if let Some(value) = take_flag_value(args, "--pids-max") {
        return value
            .parse::<u64>()
            .map_err(|_| CliError::Usage("--pids-max must be an integer".to_string()));
    }
    Ok(128)
}

fn usage_text() -> String {
    [
        "usage:",
        "  agentsnap init",
        "  agentsnap doctor [--local-only]",
        "  agentsnap exec --workspace <path> --command <command> [--isolation best_effort|hardened] [--no-shell] [--network none|restricted] [--memory-mb <n>] [--cpu-quota <n>] [--pids-max <n>]",
        "  agentsnap exec --workspace <path> --command-path <path> [--arg <arg>]... [--isolation best_effort|hardened]",
        "  agentsnap snap create --workspace <path>",
        "  agentsnap rollback --name <snapshot_id>",
    ]
    .join("\n")
}

#[derive(Debug)]
pub enum CliError {
    Usage(String),
    Runtime(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(message) => write!(f, "{message}"),
            CliError::Runtime(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::{collect_repeated_values, has_flag, take_flag_value, usage_text};

    #[test]
    fn parses_flag_values() {
        let args = vec![
            "exec".to_string(),
            "--workspace".to_string(),
            ".".to_string(),
            "--command".to_string(),
            "npm test".to_string(),
        ];

        assert_eq!(take_flag_value(&args, "--workspace"), Some(".".to_string()));
        assert_eq!(take_flag_value(&args, "--command"), Some("npm test".to_string()));
        assert_eq!(take_flag_value(&args, "--missing"), None);
    }

    #[test]
    fn usage_includes_main_commands() {
        let usage = usage_text();
        assert!(usage.contains("agentsnap init"));
        assert!(usage.contains("agentsnap doctor"));
        assert!(usage.contains("agentsnap exec"));
        assert!(usage.contains("agentsnap snap create"));
        assert!(usage.contains("agentsnap rollback"));
    }

    #[test]
    fn parses_repeated_arg_flags() {
        let args = vec![
            "--arg".to_string(),
            "a".to_string(),
            "--arg".to_string(),
            "b".to_string(),
        ];

        assert_eq!(collect_repeated_values(&args, "--arg"), vec!["a", "b"]);
        assert!(has_flag(&args, "--arg"));
    }
}
