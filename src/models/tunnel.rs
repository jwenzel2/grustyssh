use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TunnelType {
    LocalForward,
}

impl std::fmt::Display for TunnelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TunnelType::LocalForward => write!(f, "Local Forward"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub id: Uuid,
    pub name: String,
    pub tunnel_type: TunnelType,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub enabled: bool,
}

impl TunnelConfig {
    pub fn new(name: String, local_port: u16, remote_host: String, remote_port: u16) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            tunnel_type: TunnelType::LocalForward,
            local_host: "127.0.0.1".into(),
            local_port,
            remote_host,
            remote_port,
            enabled: true,
        }
    }
}
