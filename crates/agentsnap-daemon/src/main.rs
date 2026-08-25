mod handler;
mod server;

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = resolve_socket_path();
    server::run_server(&socket_path)?;
    Ok(())
}

fn resolve_socket_path() -> PathBuf {
    if let Ok(value) = env::var("AGENTSNAP_SOCKET") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--socket" {
            if let Some(value) = args.next() {
                if !value.trim().is_empty() {
                    return PathBuf::from(value);
                }
            }
        }
    }

    PathBuf::from("/tmp/agentsnap.sock")
}
