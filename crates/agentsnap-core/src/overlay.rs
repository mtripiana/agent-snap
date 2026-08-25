use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use nix::errno::Errno;
#[cfg(target_os = "linux")]
use nix::mount::{mount, umount2, MntFlags, MsFlags};

use crate::errors::{AgentSnapError, Result};

#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    pub base_dir: PathBuf,
    pub scratch_dir: PathBuf,
    pub read_only: bool,
}

#[derive(Debug)]
pub struct SnapshotSession {
    pub id: String,
    pub upper_dir: PathBuf,
    pub work_dir: PathBuf,
    pub merged_dir: PathBuf,
    pub created_at_ms: u64,
    root_dir: PathBuf,
    mounted_overlay: bool,
    read_only: bool,
    cleaned: bool,
}

impl SnapshotSession {
    /// Creates a sub-5ms Copy-On-Write scratchpad using Linux OverlayFS.
    /// Falls back to a fast shadow-copy directory when OverlayFS is unavailable.
    pub fn create(config: &SnapshotConfig) -> Result<Self> {
        validate_config(config)?;

        let id = generate_session_id();
        let root_dir = config.scratch_dir.join(&id);
        let upper_dir = root_dir.join("upper");
        let work_dir = root_dir.join("work");
        let merged_dir = root_dir.join("merged");

        fs::create_dir_all(&config.scratch_dir)?;
        set_private_permissions(&config.scratch_dir)?;
        fs::create_dir_all(&root_dir)?;
        set_private_permissions(&root_dir)?;
        fs::create_dir_all(&upper_dir)?;
        fs::create_dir_all(&work_dir)?;
        fs::create_dir_all(&merged_dir)?;

        let setup_result = (|| {
            let mounted_overlay = try_mount_overlay(config, &upper_dir, &work_dir, &merged_dir)?;
            let setup = (|| {
                if !mounted_overlay {
                    copy_tree(&config.base_dir, &merged_dir)?;
                }

                if config.read_only && !mounted_overlay {
                    apply_read_only(&merged_dir)?;
                }

                Ok(Self {
                    id,
                    upper_dir,
                    work_dir,
                    merged_dir: merged_dir.clone(),
                    created_at_ms: current_time_ms(),
                    root_dir: root_dir.clone(),
                    mounted_overlay,
                    read_only: config.read_only,
                    cleaned: false,
                })
            })();

            if setup.is_err() && mounted_overlay {
                let _ = umount_overlay(&merged_dir);
            }

            setup
        })();

        if setup_result.is_err() {
            let _ = fs::remove_dir_all(&root_dir);
        }

        setup_result
    }

    /// Instantly unmounts and purges the scratchpad.
    pub fn rollback(mut self) -> Result<()> {
        self.cleanup()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }

        if self.mounted_overlay {
            umount_overlay(&self.merged_dir)?;
            self.mounted_overlay = false;
        }

        if self.root_dir.exists() {
            make_writable(&self.root_dir)?;
            fs::remove_dir_all(&self.root_dir)?;
        }

        self.cleaned = true;
        Ok(())
    }
}

impl Drop for SnapshotSession {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn validate_config(config: &SnapshotConfig) -> Result<()> {
    if !config.base_dir.exists() {
        return Err(AgentSnapError::InvalidConfig(format!(
            "base_dir does not exist: {}",
            config.base_dir.display()
        )));
    }

    if !config.base_dir.is_dir() {
        return Err(AgentSnapError::InvalidConfig(format!(
            "base_dir is not a directory: {}",
            config.base_dir.display()
        )));
    }

    let base_dir = fs::canonicalize(&config.base_dir)?;
    let scratch_dir = if config.scratch_dir.exists() {
        fs::canonicalize(&config.scratch_dir)?
    } else {
        let parent = config
            .scratch_dir
            .parent()
            .ok_or_else(|| AgentSnapError::InvalidConfig("scratch_dir has no parent".to_string()))?;
        fs::canonicalize(parent)?.join(
            config
                .scratch_dir
                .file_name()
                .ok_or_else(|| AgentSnapError::InvalidConfig("scratch_dir has no filename".to_string()))?,
        )
    };

    if base_dir.starts_with(&scratch_dir) || scratch_dir.starts_with(&base_dir) {
        return Err(AgentSnapError::InvalidConfig(
            "base_dir and scratch_dir must not overlap".to_string(),
        ));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn try_mount_overlay(
    config: &SnapshotConfig,
    upper_dir: &Path,
    work_dir: &Path,
    merged_dir: &Path,
) -> Result<bool> {
    let options = if config.read_only {
        format!("lowerdir={}", config.base_dir.display())
    } else {
        format!(
            "lowerdir={},upperdir={},workdir={}",
            config.base_dir.display(),
            upper_dir.display(),
            work_dir.display()
        )
    };

    let flags = if config.read_only {
        MsFlags::MS_RDONLY
    } else {
        MsFlags::empty()
    };

    match mount(
        Some("overlay"),
        merged_dir,
        Some("overlay"),
        flags,
        Some(options.as_str()),
    ) {
        Ok(_) => Ok(true),
        Err(err) if should_fallback_from_mount_error(err) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

#[cfg(not(target_os = "linux"))]
fn try_mount_overlay(
    _config: &SnapshotConfig,
    _upper_dir: &Path,
    _work_dir: &Path,
    _merged_dir: &Path,
) -> Result<bool> {
    Ok(false)
}

#[cfg(target_os = "linux")]
fn should_fallback_from_mount_error(err: nix::Error) -> bool {
    match err {
        Errno::EPERM | Errno::EACCES | Errno::ENODEV | Errno::EINVAL | Errno::ENOTSUP => true,
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn umount_overlay(merged_dir: &Path) -> Result<()> {
    umount2(merged_dir, MntFlags::MNT_DETACH)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn umount_overlay(_merged_dir: &Path) -> Result<()> {
    Ok(())
}

fn copy_tree(source_root: &Path, destination_root: &Path) -> Result<()> {
    for entry in fs::read_dir(source_root)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination_root.join(entry.file_name());
        copy_entry(&source_path, &destination_path)?;
    }

    Ok(())
}

fn copy_entry(source_path: &Path, destination_path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source_path)?;

    if metadata.is_dir() {
        fs::create_dir_all(destination_path)?;
        for child in fs::read_dir(source_path)? {
            let child = child?;
            let child_source = child.path();
            let child_destination = destination_path.join(child.file_name());
            copy_entry(&child_source, &child_destination)?;
        }
        return Ok(());
    }

    if metadata.is_file() {
        fs::copy(source_path, destination_path)?;
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        if metadata.file_type().is_symlink() {
            let link_target = fs::read_link(source_path)?;
            symlink(link_target, destination_path)?;
        }
    }

    Ok(())
}

fn apply_read_only(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    set_read_only(root)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(root)? {
            apply_read_only(&entry?.path())?;
        }
    }

    Ok(())
}

fn set_read_only(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    let mut permissions = metadata.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

fn make_writable(root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }

    let mut permissions = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(root, permissions)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(root)? {
            make_writable(&entry?.path())?;
        }
    }

    Ok(())
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn generate_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let millis = current_time_ms();
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("snap_{millis}_{serial}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_config(base_dir: &Path, scratch_dir: &Path) -> SnapshotConfig {
        SnapshotConfig {
            base_dir: base_dir.to_path_buf(),
            scratch_dir: scratch_dir.to_path_buf(),
            read_only: false,
        }
    }

    #[test]
    fn create_and_rollback_snapshot() {
        let workspace = tempdir().expect("workspace tempdir");
        let scratch = tempdir().expect("scratch tempdir");

        fs::write(workspace.path().join("hello.txt"), "base").expect("write base file");

        let session = SnapshotSession::create(&create_config(workspace.path(), scratch.path()))
            .expect("create snapshot");

        let merged_file = session.merged_dir.join("hello.txt");
        assert!(merged_file.exists());

        fs::write(&merged_file, "changed").expect("write merged file");

        let base_content = fs::read_to_string(workspace.path().join("hello.txt")).expect("read base file");
        assert_eq!(base_content, "base");

        let session_root = scratch.path().join(&session.id);
        assert!(session_root.exists());

        session.rollback().expect("rollback snapshot");
        assert!(!session_root.exists());
    }

    #[test]
    fn rejects_nonexistent_base_dir() {
        let scratch = tempdir().expect("scratch tempdir");
        let missing_base = scratch.path().join("missing");

        let config = SnapshotConfig {
            base_dir: missing_base,
            scratch_dir: scratch.path().to_path_buf(),
            read_only: false,
        };

        let result = SnapshotSession::create(&config);
        assert!(matches!(result, Err(AgentSnapError::InvalidConfig(_))));
    }

    #[test]
    fn rejects_scratch_dir_inside_workspace() {
        let workspace = tempdir().expect("workspace tempdir");
        let result = SnapshotSession::create(&SnapshotConfig {
            base_dir: workspace.path().to_path_buf(),
            scratch_dir: workspace.path().join("scratch"),
            read_only: false,
        });

        assert!(matches!(result, Err(AgentSnapError::InvalidConfig(_))));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let workspace = tempdir().expect("workspace tempdir");
        let scratch = tempdir().expect("scratch tempdir");
        let external = tempdir().expect("external tempdir");
        let external_file = external.path().join("external.txt");
        fs::write(&external_file, "external").expect("write external file");
        symlink(&external_file, workspace.path().join("link.txt")).expect("create symlink");

        let session = SnapshotSession::create(&SnapshotConfig {
            base_dir: workspace.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
            read_only: true,
        })
        .expect("create snapshot");

        assert!(!fs::metadata(&external_file).expect("stat external file").permissions().readonly());
        session.rollback().expect("rollback snapshot");
    }
}
