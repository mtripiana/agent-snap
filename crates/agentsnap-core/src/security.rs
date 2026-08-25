#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationMode {
    BestEffort,
    Hardened,
}

impl IsolationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            IsolationMode::BestEffort => "best_effort",
            IsolationMode::Hardened => "hardened",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    None,
    Restricted,
}

impl NetworkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkMode::None => "none",
            NetworkMode::Restricted => "restricted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellMode {
    Allow,
    Disallow,
}

impl ShellMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellMode::Allow => "allow",
            ShellMode::Disallow => "disallow",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub memory_limit_bytes: u64,
    pub cpu_quota_percent: u64,
    pub pids_max: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 512 * 1024 * 1024,
            cpu_quota_percent: 100,
            pids_max: 128,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub isolation_mode: IsolationMode,
    pub network_mode: NetworkMode,
    pub shell_mode: ShellMode,
    pub resource_limits: ResourceLimits,
    pub seccomp_profile: String,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            isolation_mode: IsolationMode::BestEffort,
            network_mode: NetworkMode::Restricted,
            shell_mode: ShellMode::Allow,
            resource_limits: ResourceLimits::default(),
            seccomp_profile: "default_script".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnforcementReport {
    pub requested_isolation: IsolationMode,
    pub effective_isolation: IsolationMode,
    pub hardening_enforced: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SecurityCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct SecurityPreflightReport {
    pub hardened_supported: bool,
    pub checks: Vec<SecurityCheck>,
}

/// Returns the verified Linux sandbox runtime used for hardened execution.
///
/// The runtime is intentionally discovered at execution time so installations
/// can provide it through a package manager or `AGENTSNAP_BWRAP`.
pub fn hardened_runtime_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        if let Some(explicit) = std::env::var_os("AGENTSNAP_BWRAP") {
            let path = PathBuf::from(explicit);
            return is_executable(&path).then_some(path);
        }

        let path_value = std::env::var_os("PATH")?;
        for directory in std::env::split_paths(&path_value) {
            let candidate = directory.join("bwrap");
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }

        None
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub fn preflight_hardened_report() -> SecurityPreflightReport {
    let mut checks = Vec::new();

    #[cfg(target_os = "linux")]
    {
        checks.push(check_path_exists("namespaces", Path::new("/proc/self/ns")));
        checks.push(check_user_namespace_support());
        checks.push(check_seccomp_support());
        checks.push(check_cgroup_v2_support());
        checks.push(check_bwrap_support());
    }

    #[cfg(not(target_os = "linux"))]
    {
        checks.push(SecurityCheck {
            name: "platform".to_string(),
            ok: false,
            detail: "hardened mode requires linux".to_string(),
        });
    }

    let hardened_supported = checks.iter().all(|check| check.ok);
    SecurityPreflightReport {
        hardened_supported,
        checks,
    }
}

pub fn apply_security_before_exec(policy: &SecurityPolicy, rootfs: &Path) -> io::Result<()> {
    match policy.isolation_mode {
        IsolationMode::BestEffort => apply_best_effort(policy),
        IsolationMode::Hardened => apply_hardened(policy, rootfs),
    }
}

/// Applies the resource controls that must be inherited by the hardened
/// runtime. Namespace and filesystem isolation are delegated to bubblewrap.
pub fn apply_hardened_limits(policy: &SecurityPolicy) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        apply_process_limits_strict(&policy.resource_limits)?;
        configure_cgroup_v2_strict(&policy.resource_limits)?;
        set_no_new_privs()
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = policy;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "hardened mode requires linux",
        ))
    }
}

pub fn cleanup_security_artifacts(policy: &SecurityPolicy, pid: u32) {
    if policy.isolation_mode != IsolationMode::Hardened {
        return;
    }

    #[cfg(target_os = "linux")]
    {
        let path = cgroup_path_for_pid(pid);
        let _ = fs::remove_dir(path);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
    }
}

fn apply_best_effort(policy: &SecurityPolicy) -> io::Result<()> {
    apply_process_limits_best_effort(&policy.resource_limits)?;

    #[cfg(target_os = "linux")]
    {
        let _ = unshare_linux(libc::CLONE_NEWNS | libc::CLONE_NEWPID | libc::CLONE_NEWNET);
    }

    Ok(())
}

fn apply_hardened(policy: &SecurityPolicy, rootfs: &Path) -> io::Result<()> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = policy;
        let _ = rootfs;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "hardened mode requires linux",
        ))
    }

    #[cfg(target_os = "linux")]
    {
        if policy.network_mode != NetworkMode::None {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "hardened mode currently supports only network_mode=none",
            ));
        }

        apply_process_limits_strict(&policy.resource_limits)?;
        configure_cgroup_v2_strict(&policy.resource_limits)?;

        unshare_linux(libc::CLONE_NEWUSER)?;
        map_root_user_namespace()?;
        unshare_linux(
            libc::CLONE_NEWNS
                | libc::CLONE_NEWPID
                | libc::CLONE_NEWNET
                | libc::CLONE_NEWIPC
                | libc::CLONE_NEWUTS,
        )?;

        make_mounts_private()?;
        set_no_new_privs()?;
        drop_all_capabilities()?;
        apply_seccomp_profile(&policy.seccomp_profile)?;

        isolate_rootfs(rootfs)?;
        mount_minimal_procfs()?;

        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file() && fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn check_bwrap_support() -> SecurityCheck {
    match hardened_runtime_path() {
        Some(path) => SecurityCheck {
            name: "bubblewrap".to_string(),
            ok: true,
            detail: format!("runtime={}", path.display()),
        },
        None => SecurityCheck {
            name: "bubblewrap".to_string(),
            ok: false,
            detail: "bwrap not found; install bubblewrap or set AGENTSNAP_BWRAP".to_string(),
        },
    }
}

fn apply_process_limits_best_effort(limits: &ResourceLimits) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = set_limit(libc::RLIMIT_AS as _, limits.memory_limit_bytes);
        let _ = set_limit(libc::RLIMIT_CPU as _, 60);
        let _ = set_limit(libc::RLIMIT_NPROC as _, limits.pids_max);
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = limits;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_process_limits_strict(limits: &ResourceLimits) -> io::Result<()> {
    set_limit(libc::RLIMIT_AS as _, limits.memory_limit_bytes)?;
    set_limit(libc::RLIMIT_CPU as _, 60)?;
    set_limit(libc::RLIMIT_NPROC as _, limits.pids_max)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn set_limit(resource: libc::c_int, value: u64) -> io::Result<()> {
    let rlim = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };

    let result = unsafe { libc::setrlimit(resource as _, &rlim) };
    if result == 0 {
        return Ok(());
    }

    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn unshare_linux(flags: libc::c_int) -> io::Result<()> {
    let result = unsafe { libc::unshare(flags) };
    if result == 0 {
        return Ok(());
    }

    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn map_root_user_namespace() -> io::Result<()> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    if Path::new("/proc/self/setgroups").exists() {
        fs::write("/proc/self/setgroups", "deny").map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("failed to disable setgroups in user namespace: {err}"),
            )
        })?;
    }

    fs::write("/proc/self/uid_map", format!("0 {} 1\n", uid)).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("failed to write uid_map for user namespace: {err}"),
        )
    })?;

    fs::write("/proc/self/gid_map", format!("0 {} 1\n", gid)).map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("failed to write gid_map for user namespace: {err}"),
        )
    })?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn make_mounts_private() -> io::Result<()> {
    let result = unsafe {
        libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            (libc::MS_REC | libc::MS_PRIVATE) as libc::c_ulong,
            std::ptr::null(),
        )
    };

    if result == 0 {
        return Ok(());
    }

    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn isolate_rootfs(rootfs: &Path) -> io::Result<()> {
    let rootfs_c = std::ffi::CString::new(rootfs.as_os_str().as_encoded_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid rootfs path for hardened chroot",
        )
    })?;

    let result = unsafe { libc::chroot(rootfs_c.as_ptr()) };
    if result != 0 {
        return Err(io::Error::new(
            io::Error::last_os_error().kind(),
            format!(
                "failed to chroot to hardened rootfs {}: {}",
                rootfs.display(),
                io::Error::last_os_error()
            ),
        ));
    }

    let chdir_result = unsafe { libc::chdir(c"/".as_ptr()) };
    if chdir_result == 0 {
        return Ok(());
    }

    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn mount_minimal_procfs() -> io::Result<()> {
    let _ = fs::create_dir_all("/proc");

    let result = unsafe {
        libc::mount(
            c"proc".as_ptr(),
            c"/proc".as_ptr(),
            c"proc".as_ptr(),
            (libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC) as libc::c_ulong,
            std::ptr::null(),
        )
    };

    if result == 0 {
        return Ok(());
    }

    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn set_no_new_privs() -> io::Result<()> {
    let result = unsafe {
        libc::prctl(
            libc::PR_SET_NO_NEW_PRIVS,
            1,
            0,
            0,
            0,
        )
    };

    if result == 0 {
        return Ok(());
    }

    Err(io::Error::last_os_error())
}

#[cfg(target_os = "linux")]
fn drop_all_capabilities() -> io::Result<()> {
    let header = libc::__user_cap_header_struct {
        version: libc::_LINUX_CAPABILITY_VERSION_3,
        pid: 0,
    };

    let data = [
        libc::__user_cap_data_struct {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
        libc::__user_cap_data_struct {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
    ];

    let capset_result = unsafe { libc::capset(&header, data.as_ptr()) };
    if capset_result != 0 {
        return Err(io::Error::new(
            io::Error::last_os_error().kind(),
            format!("failed to clear process capabilities: {}", io::Error::last_os_error()),
        ));
    }

    for capability in 0..64 {
        let drop_result = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) };
        if drop_result != 0 {
            return Err(io::Error::new(
                io::Error::last_os_error().kind(),
                format!("failed to drop capability from bounding set: {}", io::Error::last_os_error()),
            ));
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_seccomp_profile(profile: &str) -> io::Result<()> {
    let denied = denied_syscalls_for_profile(profile)?;

    let mut instructions = Vec::with_capacity(denied.len() * 2 + 2);
    instructions.push(libc::sock_filter {
        code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
        jt: 0,
        jf: 0,
        k: std::mem::offset_of!(libc::seccomp_data, nr) as u32,
    });

    for syscall_number in denied {
        instructions.push(libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: syscall_number as u32,
        });
        instructions.push(libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_KILL_PROCESS,
        });
    }

    instructions.push(libc::sock_filter {
        code: (libc::BPF_RET | libc::BPF_K) as u16,
        jt: 0,
        jf: 0,
        k: libc::SECCOMP_RET_ALLOW,
    });

    let program = libc::sock_fprog {
        len: instructions.len() as u16,
        filter: instructions.as_mut_ptr(),
    };

    let result = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &program,
            0,
            0,
        )
    };

    if result == 0 {
        return Ok(());
    }

    Err(io::Error::new(
        io::Error::last_os_error().kind(),
        format!("failed to install seccomp profile `{profile}`: {}", io::Error::last_os_error()),
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn seccomp_filter_bytes(profile: &str) -> io::Result<Vec<u8>> {
    let denied = denied_syscalls_for_profile(profile)?;
    let instruction_count = denied.len().saturating_mul(2).saturating_add(2);
    let mut bytes = Vec::with_capacity(instruction_count * 8);

    append_bpf_instruction(
        &mut bytes,
        (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
        0,
        0,
        std::mem::offset_of!(libc::seccomp_data, nr) as u32,
    );

    for syscall_number in denied {
        append_bpf_instruction(
            &mut bytes,
            (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            0,
            1,
            syscall_number as u32,
        );
        append_bpf_instruction(
            &mut bytes,
            (libc::BPF_RET | libc::BPF_K) as u16,
            0,
            0,
            libc::SECCOMP_RET_KILL_PROCESS,
        );
    }

    append_bpf_instruction(
        &mut bytes,
        (libc::BPF_RET | libc::BPF_K) as u16,
        0,
        0,
        libc::SECCOMP_RET_ALLOW,
    );

    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn append_bpf_instruction(bytes: &mut Vec<u8>, code: u16, jt: u8, jf: u8, k: u32) {
    bytes.extend_from_slice(&code.to_ne_bytes());
    bytes.push(jt);
    bytes.push(jf);
    bytes.extend_from_slice(&k.to_ne_bytes());
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn seccomp_filter_bytes(_profile: &str) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "seccomp requires linux",
    ))
}

#[cfg(target_os = "linux")]
fn denied_syscalls_for_profile(profile: &str) -> io::Result<Vec<i64>> {
    let common = vec![
        libc::SYS_ptrace,
        libc::SYS_setns,
        libc::SYS_unshare,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_kexec_load,
    ];

    match profile {
        "default_script" => Ok(common),
        "default_build" => {
            let mut values = common;
            values.push(libc::SYS_bpf);
            Ok(values)
        }
        value => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown seccomp profile: {value}"),
        )),
    }
}

#[cfg(target_os = "linux")]
fn configure_cgroup_v2_strict(limits: &ResourceLimits) -> io::Result<()> {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    if !cgroup_root.join("cgroup.controllers").exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "cgroup v2 controllers not available",
        ));
    }

    let parent = cgroup_root.join("agentsnap");
    fs::create_dir_all(&parent)?;

    let pid = unsafe { libc::getpid() } as u32;
    let child_path = cgroup_path_for_pid(pid);
    fs::create_dir_all(&child_path)?;

    fs::write(
        child_path.join("memory.max"),
        limits.memory_limit_bytes.to_string(),
    )?;

    let period = 100_000u64;
    let quota = ((limits.cpu_quota_percent.max(1)).saturating_mul(period)) / 100;
    fs::write(child_path.join("cpu.max"), format!("{} {}", quota, period))?;

    fs::write(child_path.join("pids.max"), limits.pids_max.to_string())?;
    fs::write(child_path.join("cgroup.procs"), pid.to_string())?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn cgroup_path_for_pid(pid: u32) -> std::path::PathBuf {
    Path::new("/sys/fs/cgroup")
        .join("agentsnap")
        .join(pid.to_string())
}

#[cfg(target_os = "linux")]
fn check_user_namespace_support() -> SecurityCheck {
    let path = Path::new("/proc/sys/user/max_user_namespaces");
    if !path.exists() {
        return SecurityCheck {
            name: "userns".to_string(),
            ok: false,
            detail: "kernel does not expose max_user_namespaces".to_string(),
        };
    }

    match fs::read_to_string(path) {
        Ok(value) => {
            let parsed = value.trim().parse::<u64>().unwrap_or(0);
            SecurityCheck {
                name: "userns".to_string(),
                ok: parsed > 0,
                detail: format!("max_user_namespaces={parsed}"),
            }
        }
        Err(err) => SecurityCheck {
            name: "userns".to_string(),
            ok: false,
            detail: format!("cannot read max_user_namespaces: {err}"),
        },
    }
}

#[cfg(target_os = "linux")]
fn check_seccomp_support() -> SecurityCheck {
    let actions_file = Path::new("/proc/sys/kernel/seccomp/actions_avail");
    if !actions_file.exists() {
        return SecurityCheck {
            name: "seccomp".to_string(),
            ok: false,
            detail: "seccomp actions file missing".to_string(),
        };
    }

    let detail = fs::read_to_string(actions_file).unwrap_or_else(|_| "unknown".to_string());
    SecurityCheck {
        name: "seccomp".to_string(),
        ok: true,
        detail: format!("actions_avail={}", detail.trim()),
    }
}

#[cfg(target_os = "linux")]
fn check_cgroup_v2_support() -> SecurityCheck {
    let root = Path::new("/sys/fs/cgroup");
    let controllers = root.join("cgroup.controllers");
    if !controllers.exists() {
        return SecurityCheck {
            name: "cgroup_v2".to_string(),
            ok: false,
            detail: "cgroup v2 controllers file missing".to_string(),
        };
    }

    let writable_probe = OpenOptions::new().write(true).open(root.join("cgroup.procs"));
    match writable_probe {
        Ok(_) => SecurityCheck {
            name: "cgroup_v2".to_string(),
            ok: true,
            detail: "cgroup.procs writable".to_string(),
        },
        Err(err) => SecurityCheck {
            name: "cgroup_v2".to_string(),
            ok: false,
            detail: format!("cannot write cgroup.procs: {err}"),
        },
    }
}

#[cfg(target_os = "linux")]
fn check_path_exists(name: &str, path: &Path) -> SecurityCheck {
    SecurityCheck {
        name: name.to_string(),
        ok: path.exists(),
        detail: format!("path={}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_best_effort() {
        let policy = SecurityPolicy::default();
        assert_eq!(policy.isolation_mode, IsolationMode::BestEffort);
        assert_eq!(policy.shell_mode, ShellMode::Allow);
    }

    #[test]
    fn preflight_returns_checks() {
        let report = preflight_hardened_report();
        assert!(!report.checks.is_empty());
    }

    #[test]
    fn seccomp_profile_requires_known_name() {
        #[cfg(target_os = "linux")]
        {
            let result = denied_syscalls_for_profile("unknown_profile");
            assert!(result.is_err());
        }
    }
}
