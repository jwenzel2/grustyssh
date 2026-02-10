use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("SSH error: {0}")]
    Ssh(#[from] russh::Error),

    #[error("SSH key error: {0}")]
    SshKey(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Tunnel error: {0}")]
    Tunnel(String),

    #[error("Key generation error: {0}")]
    KeyGen(String),

    #[error("Host key verification failed: {0}")]
    HostKey(String),

    #[error("{0}")]
    Other(String),
}

impl From<ssh_key::Error> for AppError {
    fn from(e: ssh_key::Error) -> Self {
        AppError::SshKey(e.to_string())
    }
}
