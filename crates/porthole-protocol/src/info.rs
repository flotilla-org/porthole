use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InfoResponse {
    pub daemon_version: String,
    pub uptime_seconds: u64,
    pub adapters: Vec<AdapterInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub name: String,
    pub loaded: bool,
    pub capabilities: Vec<String>,
}
