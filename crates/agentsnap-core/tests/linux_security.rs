#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::fs;

use agentsnap_core::{
    execute_in_sandbox, preflight_hardened_report, ExecutionOptions, IsolationMode, NetworkMode,
    SecurityPolicy, ShellMode, SnapshotConfig, SnapshotSession,
};
use tempfile::{tempdir, TempDir};

fn hardened_session() -> (TempDir, TempDir, SnapshotSession) {
    let report = preflight_hardened_report();
    assert!(
        report.hardened_supported,
        "Linux hardened preflight failed: {}",
        report
            .checks
            .iter()
            .filter(|check| !check.ok)
            .map(|check| format!("{}={}", check.name, check.detail))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let workspace = tempdir().expect("workspace");
    let scratch = tempdir().expect("scratch");
    fs::write(workspace.path().join("input.txt"), "input").expect("seed workspace");
    let session = SnapshotSession::create(&SnapshotConfig {
        base_dir: workspace.path().to_path_buf(),
        scratch_dir: scratch.path().to_path_buf(),
        read_only: false,
    })
    .expect("hardened snapshot");

    (workspace, scratch, session)
}

fn hardened_options(args: Vec<String>, timeout_ms: u64) -> ExecutionOptions {
    ExecutionOptions {
        command: "/usr/bin/python3".to_string(),
        args,
        env: HashMap::new(),
        timeout_ms,
        policy: SecurityPolicy {
            isolation_mode: IsolationMode::Hardened,
            network_mode: NetworkMode::None,
            shell_mode: ShellMode::Disallow,
            ..SecurityPolicy::default()
        },
        is_shell_command: false,
        max_output_bytes: 64 * 1024,
    }
}

#[test]
fn hardened_mode_blocks_external_files_and_network() {
    let external = tempdir().expect("external");
    let marker = external.path().join("outside.txt");
    fs::write(&marker, "outside").expect("external marker");
    let marker = marker.to_string_lossy().replace('\\', "\\\\").replace('\'', "\\'");

    let (_workspace, _scratch, session) = hardened_session();
    let result = execute_in_sandbox(
        &session,
        hardened_options(
            vec![
                "-c".to_string(),
                format!(
                    "import os,socket; print('external-visible' if os.path.exists('{}') else 'external-hidden'); s=socket.socket(); s.settimeout(.2);\ntry: s.connect(('1.1.1.1',80)); print('network-open')\nexcept OSError: print('network-blocked')",
                    marker
                ),
            ],
            5_000,
        ),
    )
    .expect("hardened execution");

    assert!(result.stdout.contains("external-hidden"), "stdout={:?}", result.stdout);
    assert!(result.stdout.contains("network-blocked"), "stdout={:?}", result.stdout);
    assert_eq!(result.effective_isolation, IsolationMode::Hardened);
    session.rollback().expect("cleanup");
}

#[test]
fn hardened_mode_kills_process_tree_on_timeout() {
    let (_workspace, _scratch, session) = hardened_session();
    let result = execute_in_sandbox(
        &session,
        hardened_options(
            vec![
                "-c".to_string(),
                "import subprocess,time; subprocess.Popen(['/usr/bin/sleep','30']); time.sleep(30)".to_string(),
            ],
            150,
        ),
    );

    assert!(matches!(result, Err(agentsnap_core::AgentSnapError::Timeout { .. })));
    session.rollback().expect("cleanup");
}

#[test]
fn hardened_mode_applies_seccomp_profile() {
    let (_workspace, _scratch, session) = hardened_session();
    let result = execute_in_sandbox(
        &session,
        hardened_options(
            vec![
                "-c".to_string(),
                "import ctypes; libc=ctypes.CDLL(None); libc.mount(None,b'/tmp',None,0,None)".to_string(),
            ],
            5_000,
        ),
    )
    .expect("hardened execution");

    assert_ne!(result.exit_code, 0);
    assert!(result.enforcement_report.hardening_enforced);
    session.rollback().expect("cleanup");
}
