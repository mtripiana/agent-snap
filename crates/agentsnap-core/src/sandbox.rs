use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::errors::{AgentSnapError, Result};
use crate::overlay::SnapshotSession;
use crate::security::{
    apply_hardened_limits, apply_security_before_exec, cleanup_security_artifacts,
    preflight_hardened_report, seccomp_filter_bytes, EnforcementReport,
    hardened_runtime_path, IsolationMode, SecurityPolicy, ShellMode,
};

const DEFAULT_MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub timeout_ms: u64,
    pub policy: SecurityPolicy,
    pub is_shell_command: bool,
    pub max_output_bytes: usize,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            timeout_ms: 30_000,
            policy: SecurityPolicy::default(),
            is_shell_command: false,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub effective_isolation: IsolationMode,
    pub enforcement_report: EnforcementReport,
}

pub fn execute_in_sandbox(session: &SnapshotSession, opts: ExecutionOptions) -> Result<ExecutionResult> {
    validate_execution_options(&opts)?;

    if opts.policy.isolation_mode == IsolationMode::Hardened {
        let preflight = preflight_hardened_report();
        if !preflight.hardened_supported {
            return Err(AgentSnapError::CapabilityUnavailable {
                component: "hardened_preflight".to_string(),
                detail: format_preflight_failures(&preflight),
            });
        }
    }

    let started = Instant::now();
    let timeout = Duration::from_millis(opts.timeout_ms);

    let (program, command_args) = if opts.policy.isolation_mode == IsolationMode::Hardened {
        let runtime = hardened_runtime_path().ok_or_else(|| AgentSnapError::CapabilityUnavailable {
            component: "hardened_runtime".to_string(),
            detail: "bubblewrap runtime is unavailable".to_string(),
        })?;
        (runtime, Vec::new())
    } else {
        (std::path::PathBuf::from(&opts.command), opts.args.clone())
    };

    let seccomp_file = if opts.policy.isolation_mode == IsolationMode::Hardened {
        Some(prepare_seccomp_filter(&opts.policy.seccomp_profile)?)
    } else {
        None
    };
    let seccomp_fd = seccomp_file.as_ref().map(|file| file.as_raw_fd());
    let command_args = if opts.policy.isolation_mode == IsolationMode::Hardened {
        hardened_command_args(session, &opts, seccomp_fd)
    } else {
        command_args
    };

    let mut command = Command::new(program);
    command
        .args(command_args)
        .current_dir(&session.merged_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .env_clear();

    if opts.policy.isolation_mode == IsolationMode::Hardened {
        command.env_clear();
    } else {
        command.env("PATH", "/usr/bin:/bin");
        for (key, value) in &opts.env {
            command.env(key, value);
        }
    }

    let policy = opts.policy.clone();
    let rootfs = session.merged_dir.clone();
    let seccomp_fd_for_child = seccomp_fd;

    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }

            if policy.isolation_mode == IsolationMode::Hardened {
                if let Some(fd) = seccomp_fd_for_child {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                apply_hardened_limits(&policy)
            } else {
                apply_security_before_exec(&policy, &rootfs)
            }
        });
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            cleanup_security_artifacts(&opts.policy, 0);
            return Err(err.into());
        }
    };
    let child_pid = child.id();
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let stderr = child.stderr.take().expect("stderr was configured as piped");
    let stdout_reader = spawn_capped_reader(stdout, opts.max_output_bytes);
    let stderr_reader = spawn_capped_reader(stderr, opts.max_output_bytes);

    loop {
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(err) => {
                kill_process_group(child_pid);
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                cleanup_security_artifacts(&opts.policy, child_pid);
                return Err(err.into());
            }
        };

        if let Some(status) = status {
            // A shell can exit while background descendants keep the pipes
            // open. Tear down the process group before waiting for captures.
            kill_process_group(child_pid);
            let stdout = stdout_reader.join().map_err(|_| {
                AgentSnapError::Io(std::io::Error::other("stdout capture thread panicked"))
            })?;
            let stderr = stderr_reader.join().map_err(|_| {
                AgentSnapError::Io(std::io::Error::other("stderr capture thread panicked"))
            })?;
            cleanup_security_artifacts(&opts.policy, child_pid);

            let effective_isolation = opts.policy.isolation_mode;
            let enforcement_report = EnforcementReport {
                requested_isolation: opts.policy.isolation_mode,
                effective_isolation,
                hardening_enforced: effective_isolation == IsolationMode::Hardened,
                notes: build_enforcement_notes(
                    &opts.policy,
                    started.elapsed().as_millis() as u64,
                    stdout.truncated,
                    stderr.truncated,
                ),
            };

            return Ok(ExecutionResult {
                exit_code: status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
                stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
                duration_ms: started.elapsed().as_millis() as u64,
                effective_isolation,
                enforcement_report,
            });
        }

        if started.elapsed() >= timeout {
            kill_process_group(child_pid);
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            cleanup_security_artifacts(&opts.policy, child_pid);

            return Err(AgentSnapError::Timeout {
                timeout_ms: opts.timeout_ms,
            });
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn validate_execution_options(opts: &ExecutionOptions) -> Result<()> {
    if opts.command.trim().is_empty() {
        return Err(AgentSnapError::InvalidExecutionOptions(
            "command cannot be empty".to_string(),
        ));
    }

    if opts.timeout_ms == 0 {
        return Err(AgentSnapError::InvalidExecutionOptions(
            "timeout_ms must be greater than 0".to_string(),
        ));
    }

    if opts.max_output_bytes == 0 {
        return Err(AgentSnapError::InvalidExecutionOptions(
            "max_output_bytes must be greater than 0".to_string(),
        ));
    }

    if opts.policy.resource_limits.memory_limit_bytes == 0
        || opts.policy.resource_limits.cpu_quota_percent == 0
        || opts.policy.resource_limits.pids_max == 0
    {
        return Err(AgentSnapError::InvalidExecutionOptions(
            "resource limits must be greater than 0".to_string(),
        ));
    }

    if opts.policy.isolation_mode == IsolationMode::Hardened
        && opts.policy.shell_mode == ShellMode::Disallow
        && opts.is_shell_command
    {
        return Err(AgentSnapError::PolicyViolation(
            "shell command strings are disallowed in hardened mode; use command_path + args".to_string(),
        ));
    }

    Ok(())
}

fn format_preflight_failures(report: &crate::security::SecurityPreflightReport) -> String {
    report
        .checks
        .iter()
        .filter(|check| !check.ok)
        .map(|check| format!("{}: {}", check.name, check.detail))
        .collect::<Vec<_>>()
        .join("; ")
}

fn build_enforcement_notes(
    policy: &SecurityPolicy,
    duration_ms: u64,
    stdout_truncated: bool,
    stderr_truncated: bool,
) -> Vec<String> {
    let mut notes = Vec::new();
    notes.push(format!(
        "network_mode={} shell_mode={} seccomp_profile={}",
        policy.network_mode.as_str(),
        policy.shell_mode.as_str(),
        policy.seccomp_profile,
    ));
    notes.push(format!(
        "limits memory={} cpu_percent={} pids_max={}",
        policy.resource_limits.memory_limit_bytes,
        policy.resource_limits.cpu_quota_percent,
        policy.resource_limits.pids_max
    ));
    notes.push(format!("duration_ms={duration_ms}"));
    if stdout_truncated {
        notes.push("stdout_truncated=true".to_string());
    }
    if stderr_truncated {
        notes.push("stderr_truncated=true".to_string());
    }
    notes
}

fn spawn_capped_reader<R>(mut reader: R, limit: usize) -> JoinHandle<CapturedOutput>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if bytes.len() < limit {
                        let remaining = limit - bytes.len();
                        bytes.extend_from_slice(&buffer[..read.min(remaining)]);
                    }
                    if bytes.len() >= limit && read > limit.saturating_sub(bytes.len()) {
                        truncated = true;
                    }
                }
                Err(_) => break,
            }
        }

        CapturedOutput { bytes, truncated }
    })
}

#[derive(Debug)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn kill_process_group(pid: u32) {
    unsafe {
        let _ = libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
    }
}

fn prepare_seccomp_filter(profile: &str) -> Result<File> {
    let bytes = seccomp_filter_bytes(profile)?;
    let path = std::env::temp_dir().join(format!(
        "agentsnap-seccomp-{}-{}",
        std::process::id(),
        started_filter_id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)?;

    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.seek(SeekFrom::Start(0))?;
    let _ = fs::remove_file(path);
    Ok(file)
}

fn started_filter_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILTER_ID: AtomicU64 = AtomicU64::new(0);
    NEXT_FILTER_ID.fetch_add(1, Ordering::Relaxed)
}

fn hardened_command_args(
    session: &SnapshotSession,
    opts: &ExecutionOptions,
    seccomp_fd: Option<RawFd>,
) -> Vec<String> {
    let mut args = vec![
        "--die-with-parent".to_string(),
        "--new-session".to_string(),
        "--unshare-user".to_string(),
        "--disable-userns".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-net".to_string(),
        "--unshare-ipc".to_string(),
        "--unshare-uts".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--tmpfs".to_string(),
        "/tmp".to_string(),
        "--ro-bind".to_string(),
        "/usr".to_string(),
        "/usr".to_string(),
        "--ro-bind".to_string(),
        "/bin".to_string(),
        "/bin".to_string(),
        "--ro-bind-try".to_string(),
        "/lib".to_string(),
        "/lib".to_string(),
        "--ro-bind-try".to_string(),
        "/lib64".to_string(),
        "/lib64".to_string(),
        "--ro-bind-try".to_string(),
        "/etc".to_string(),
        "/etc".to_string(),
    ];

    args.extend([
        if session.is_read_only() { "--ro-bind" } else { "--bind" }.to_string(),
        session.merged_dir.display().to_string(),
        "/workspace".to_string(),
        "--chdir".to_string(),
        "/workspace".to_string(),
        "--clearenv".to_string(),
        "--setenv".to_string(),
        "PATH".to_string(),
        "/usr/bin:/bin".to_string(),
    ]);

    for (key, value) in &opts.env {
        args.extend(["--setenv".to_string(), key.clone(), value.clone()]);
    }

    if let Some(fd) = seccomp_fd {
        args.extend(["--seccomp".to_string(), fd.to_string()]);
    }

    args.push("--".to_string());
    args.push(opts.command.clone());
    args.extend(opts.args.clone());
    args
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use tempfile::tempdir;

    use crate::errors::AgentSnapError;
    use crate::overlay::{SnapshotConfig, SnapshotSession};
    use crate::security::{IsolationMode, NetworkMode, SecurityPolicy, ShellMode};

    use super::{execute_in_sandbox, ExecutionOptions};

    #[test]
    fn execute_captures_stdout_stderr_and_env() {
        let workspace = tempdir().expect("workspace tempdir");
        let scratch = tempdir().expect("scratch tempdir");

        fs::write(workspace.path().join("hello.txt"), "hello-from-workspace").expect("seed file");

        let session = SnapshotSession::create(&SnapshotConfig {
            base_dir: workspace.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
            read_only: false,
        })
        .expect("create snapshot");

        let mut env = HashMap::new();
        env.insert("AGENTSNAP_TEST_ENV".to_string(), "env-value".to_string());

        let result = execute_in_sandbox(
            &session,
            ExecutionOptions {
                command: "/bin/sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    "cat hello.txt; echo $AGENTSNAP_TEST_ENV; echo err-line 1>&2".to_string(),
                ],
                env,
                timeout_ms: 2_000,
                policy: SecurityPolicy::default(),
                is_shell_command: true,
                max_output_bytes: 1024 * 1024,
            },
        )
        .expect("execute");

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello-from-workspace"));
        assert!(result.stdout.contains("env-value"));
        assert!(result.stderr.contains("err-line"));
        assert!(result.duration_ms > 0);
        assert_eq!(result.effective_isolation, IsolationMode::BestEffort);

        session.rollback().expect("rollback snapshot");
    }

    #[test]
    fn execute_returns_timeout_error() {
        let workspace = tempdir().expect("workspace tempdir");
        let scratch = tempdir().expect("scratch tempdir");

        let session = SnapshotSession::create(&SnapshotConfig {
            base_dir: workspace.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
            read_only: false,
        })
        .expect("create snapshot");

        let result = execute_in_sandbox(
            &session,
            ExecutionOptions {
                command: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "sleep 2".to_string()],
                env: HashMap::new(),
                timeout_ms: 100,
                policy: SecurityPolicy::default(),
                is_shell_command: true,
                max_output_bytes: 1024 * 1024,
            },
        );

        assert!(matches!(result, Err(AgentSnapError::Timeout { .. })));

        session.rollback().expect("rollback snapshot");
    }

    #[test]
    fn execute_caps_captured_output() {
        let workspace = tempdir().expect("workspace tempdir");
        let scratch = tempdir().expect("scratch tempdir");
        let session = SnapshotSession::create(&SnapshotConfig {
            base_dir: workspace.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
            read_only: false,
        })
        .expect("create snapshot");

        let result = execute_in_sandbox(
            &session,
            ExecutionOptions {
                command: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "printf 0123456789abcdefghij".to_string()],
                env: HashMap::new(),
                timeout_ms: 2_000,
                policy: SecurityPolicy::default(),
                is_shell_command: true,
                max_output_bytes: 8,
            },
        )
        .expect("execute");

        assert_eq!(result.stdout.len(), 8);
        assert!(result
            .enforcement_report
            .notes
            .iter()
            .any(|note| note == "stdout_truncated=true"));
        session.rollback().expect("rollback snapshot");
    }

    #[test]
    fn hardened_mode_rejects_shell_commands_by_default() {
        let workspace = tempdir().expect("workspace tempdir");
        let scratch = tempdir().expect("scratch tempdir");

        let session = SnapshotSession::create(&SnapshotConfig {
            base_dir: workspace.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
            read_only: false,
        })
        .expect("create snapshot");

        let policy = SecurityPolicy {
            isolation_mode: IsolationMode::Hardened,
            network_mode: NetworkMode::None,
            shell_mode: ShellMode::Disallow,
            ..SecurityPolicy::default()
        };

        let result = execute_in_sandbox(
            &session,
            ExecutionOptions {
                command: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), "echo blocked".to_string()],
                env: HashMap::new(),
                timeout_ms: 200,
                policy,
                is_shell_command: true,
                max_output_bytes: 1024 * 1024,
            },
        );

        assert!(matches!(result, Err(AgentSnapError::PolicyViolation(_))));
        session.rollback().expect("rollback snapshot");
    }
}
