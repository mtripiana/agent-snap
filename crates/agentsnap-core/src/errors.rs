use std::io;

#[derive(Debug, thiserror::Error)]
pub enum AgentSnapError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("nix error: {0}")]
    Nix(#[from] nix::Error),

    #[error("invalid config: {0}")]
    InvalidConfig(String),

    #[error("invalid execution options: {0}")]
    InvalidExecutionOptions(String),

    #[error("execution timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("capability unavailable: {component}: {detail}")]
    CapabilityUnavailable { component: String, detail: String },

    #[error("policy violation: {0}")]
    PolicyViolation(String),
}

pub type Result<T> = std::result::Result<T, AgentSnapError>;
