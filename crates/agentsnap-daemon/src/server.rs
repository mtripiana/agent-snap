use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::handler::{default_scratch_root, DaemonState};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

pub fn run_server(socket_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
        validate_parent_dir_security(parent)?;
    }

    if socket_path.exists() {
        let metadata = fs::symlink_metadata(socket_path)?;
        if !metadata.file_type().is_socket() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("refusing to replace non-socket path {}", socket_path.display()),
            ));
        }
        fs::remove_file(socket_path)?;
    }

    fs::create_dir_all(default_scratch_root())?;
    fs::set_permissions(default_scratch_root(), fs::Permissions::from_mode(0o700))?;
    cleanup_old_scratchpads(default_scratch_root())?;

    let previous_umask = unsafe { libc::umask(0o077) };
    let listener_result = UnixListener::bind(socket_path);
    unsafe {
        libc::umask(previous_umask);
    }
    let listener = listener_result?;
    secure_socket_permissions(socket_path)?;
    let state = Arc::new(Mutex::new(DaemonState::new(default_scratch_root())));

    for stream_result in listener.incoming() {
        let mut stream = match stream_result {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("accept error: {err}");
                continue;
            }
        };

        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;

        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            let mut state = match state.lock() {
                Ok(state) => state,
                Err(err) => {
                    eprintln!("daemon state lock poisoned: {err}");
                    return;
                }
            };

            if let Err(err) = handle_client(&mut state, &mut stream) {
                eprintln!("client handling error: {err}");
            }
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn secure_socket_permissions(socket_path: &Path) -> std::io::Result<()> {
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(target_os = "linux"))]
fn secure_socket_permissions(_socket_path: &Path) -> std::io::Result<()> {
    // Some Unix implementations reject chmod on AF_UNIX sockets. The 077
    // umask around bind above still creates the socket privately.
    Ok(())
}

fn cleanup_old_scratchpads(root: &Path) -> std::io::Result<()> {
    let now = SystemTime::now();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let age = now
            .duration_since(metadata.modified().unwrap_or(now))
            .unwrap_or_default();
        if age > Duration::from_secs(24 * 60 * 60) {
            if metadata.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
    }

    Ok(())
}

fn validate_parent_dir_security(parent: &Path) -> std::io::Result<()> {
    let metadata = fs::metadata(parent)?;
    let mode = metadata.permissions().mode();
    let world_writable = (mode & 0o002) != 0;
    let sticky_bit = (mode & 0o1000) != 0;

    if world_writable && !sticky_bit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "refusing insecure socket parent directory (world-writable without sticky bit): {}",
                parent.display()
            ),
        ));
    }

    Ok(())
}

fn handle_client(state: &mut DaemonState, stream: &mut UnixStream) -> std::io::Result<()> {
    let mut request_bytes = Vec::new();
    stream
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut request_bytes)?;

    if request_bytes.len() > MAX_REQUEST_BYTES {
        let response = br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"request exceeds maximum size"}}
"#;
        stream.write_all(response)?;
        return Ok(());
    }

    if request_bytes.is_empty() {
        return Ok(());
    }

    let response = state.handle_request_json(&request_bytes);
    let mut serialized = serde_json::to_vec(&response)?;
    serialized.push(b'\n');

    stream.write_all(&serialized)?;
    stream.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::handler::DaemonState;

    use super::{handle_client, MAX_REQUEST_BYTES};

    #[test]
    fn handle_client_writes_response() {
        let (mut server_stream, mut client_stream) = UnixStream::pair().expect("socket pair");

        let request = json!({
            "id": 1,
            "method": "agentsnap.unknown",
            "params": {}
        })
        .to_string();

        let writer = std::thread::spawn(move || {
            client_stream.write_all(request.as_bytes()).expect("write request");
            client_stream.shutdown(std::net::Shutdown::Write).expect("shutdown write");

            let mut out = String::new();
            client_stream.read_to_string(&mut out).expect("read response");
            out
        });

        let scratch = tempdir().expect("scratch");
        let mut state = DaemonState::new(scratch.path());

        handle_client(&mut state, &mut server_stream).expect("handle client");
        drop(server_stream);

        let output = writer.join().expect("join writer");
        assert!(output.contains("method not found"));
    }

    #[test]
    fn handle_client_rejects_malformed_json() {
        let (mut server_stream, mut client_stream) = UnixStream::pair().expect("socket pair");
        let writer = std::thread::spawn(move || {
            client_stream.write_all(b"not-json").expect("write request");
            client_stream.shutdown(std::net::Shutdown::Write).expect("shutdown write");
            let mut output = String::new();
            client_stream.read_to_string(&mut output).expect("read response");
            output
        });

        let scratch = tempdir().expect("scratch");
        let mut state = DaemonState::new(scratch.path());
        handle_client(&mut state, &mut server_stream).expect("handle malformed request");
        drop(server_stream);

        let output = writer.join().expect("join writer");
        assert!(output.contains("-32700"));
    }

    #[test]
    fn handle_client_rejects_oversized_request() {
        let (mut server_stream, mut client_stream) = UnixStream::pair().expect("socket pair");
        let writer = std::thread::spawn(move || {
            let request = vec![b'x'; MAX_REQUEST_BYTES + 1];
            client_stream.write_all(&request).expect("write request");
            client_stream.shutdown(std::net::Shutdown::Write).expect("shutdown write");
            let mut output = String::new();
            client_stream.read_to_string(&mut output).expect("read response");
            output
        });

        let scratch = tempdir().expect("scratch");
        let mut state = DaemonState::new(scratch.path());
        handle_client(&mut state, &mut server_stream).expect("handle oversized request");
        drop(server_stream);

        let output = writer.join().expect("join writer");
        assert!(output.contains("request exceeds maximum size"));
    }

}
