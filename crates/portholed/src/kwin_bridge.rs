use std::{
    collections::{HashMap, VecDeque},
    future,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::{
    sync::{Mutex, Notify},
    time::timeout,
};
use tracing::{info, warn};
use uuid::Uuid;
use zbus::{connection::Builder as ConnectionBuilder, fdo};

pub const SERVICE_NAME: &str = "work.flotilla.Porthole.KWin";
pub const OBJECT_PATH: &str = "/work/flotilla/Porthole/KWin";
pub const INTERFACE_NAME: &str = "work.flotilla.Porthole.KWin";
const COMMAND_LONG_POLL_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug)]
pub struct KWinBridge {
    state: Arc<Mutex<KWinBridgeState>>,
    commands_available: Arc<Notify>,
}

#[derive(Debug, Default)]
struct KWinBridgeState {
    latest_snapshot: Option<KWinSnapshot>,
    commands: VecDeque<KWinCommand>,
    completions: HashMap<String, KWinCommandCompletion>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KWinSnapshot {
    pub payload: Value,
    pub received_unix_ms: u128,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KWinCommand {
    pub command_id: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KWinCommandCompletion {
    pub command_id: String,
    pub result_json: String,
    pub completed_unix_ms: u128,
}

#[derive(Debug, Error)]
pub enum KWinBridgeError {
    #[error("invalid KWin bridge JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

impl KWinBridge {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(KWinBridgeState::default())),
            commands_available: Arc::new(Notify::new()),
        }
    }

    pub async fn publish_snapshot_json(&self, payload_json: &str) -> Result<(), KWinBridgeError> {
        let payload = serde_json::from_str(payload_json)?;
        let mut state = self.state.lock().await;
        state.latest_snapshot = Some(KWinSnapshot {
            payload,
            received_unix_ms: unix_ms(),
        });
        Ok(())
    }

    pub async fn latest_snapshot(&self) -> Option<KWinSnapshot> {
        self.state.lock().await.latest_snapshot.clone()
    }

    pub async fn queue_command(&self, kind: impl Into<String>, payload: Value) -> KWinCommand {
        let command = KWinCommand {
            command_id: Uuid::new_v4().to_string(),
            kind: kind.into(),
            payload,
        };
        self.state.lock().await.commands.push_back(command.clone());
        self.commands_available.notify_one();
        command
    }

    pub async fn next_command_json(&self, _script_instance_id: &str) -> Result<Option<String>, KWinBridgeError> {
        loop {
            if let Some(command) = self.state.lock().await.commands.pop_front() {
                return Ok(Some(serde_json::to_string(&command)?));
            }
            if timeout(COMMAND_LONG_POLL_TIMEOUT, self.commands_available.notified())
                .await
                .is_err()
            {
                return Ok(None);
            }
        }
    }

    pub async fn complete_command_json(&self, command_id: &str, result_json: &str) {
        let completion = KWinCommandCompletion {
            command_id: command_id.to_owned(),
            result_json: result_json.to_owned(),
            completed_unix_ms: unix_ms(),
        };
        self.state.lock().await.completions.insert(command_id.to_owned(), completion);
    }

    pub async fn completion(&self, command_id: &str) -> Option<KWinCommandCompletion> {
        self.state.lock().await.completions.get(command_id).cloned()
    }
}

impl Default for KWinBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[zbus::interface(name = "work.flotilla.Porthole.KWin")]
impl KWinBridge {
    #[zbus(name = "PublishSnapshot")]
    async fn dbus_publish_snapshot(&self, payload_json: &str) -> fdo::Result<()> {
        self.publish_snapshot_json(payload_json)
            .await
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        Ok(())
    }

    #[zbus(name = "NextCommand")]
    async fn dbus_next_command(&self, script_instance_id: &str) -> fdo::Result<String> {
        self.next_command_json(script_instance_id)
            .await
            .map(|command| command.unwrap_or_default())
            .map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    #[zbus(name = "CompleteCommand")]
    async fn dbus_complete_command(&self, command_id: &str, result_json: &str) -> fdo::Result<()> {
        self.complete_command_json(command_id, result_json).await;
        Ok(())
    }
}

pub fn spawn_session_service(bridge: KWinBridge) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = run_session_service(bridge).await {
            warn!(error = %error, "KWin bridge session bus service stopped");
        }
    })
}

pub async fn run_session_service(bridge: KWinBridge) -> zbus::Result<()> {
    let _connection = ConnectionBuilder::session()?
        .name(SERVICE_NAME)?
        .serve_at(OBJECT_PATH, bridge)?
        .build()
        .await?;
    info!(
        service = SERVICE_NAME,
        object_path = OBJECT_PATH,
        interface = INTERFACE_NAME,
        "KWin bridge listening"
    );
    future::pending::<()>().await;
    Ok(())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn publish_snapshot_stores_latest_valid_json() {
        let bridge = KWinBridge::new();

        bridge
            .publish_snapshot_json(r#"{"schemaVersion":1,"windowCount":2}"#)
            .await
            .unwrap();

        let snapshot = bridge.latest_snapshot().await.unwrap();
        assert_eq!(snapshot.payload["schemaVersion"], 1);
        assert_eq!(snapshot.payload["windowCount"], 2);
    }

    #[tokio::test]
    async fn publish_snapshot_rejects_invalid_json() {
        let bridge = KWinBridge::new();

        let error = bridge.publish_snapshot_json("not json").await.unwrap_err();

        assert!(matches!(error, KWinBridgeError::InvalidJson(_)));
        assert!(bridge.latest_snapshot().await.is_none());
    }

    #[tokio::test]
    async fn command_queue_returns_fifo_json_then_empty() {
        let bridge = KWinBridge::new();
        let first = bridge.queue_command("focus", json!({ "windowId": "a" })).await;
        let second = bridge.queue_command("close", json!({ "windowId": "b" })).await;

        let first_json = bridge.next_command_json("script").await.unwrap().unwrap();
        let second_json = bridge.next_command_json("script").await.unwrap().unwrap();

        assert_eq!(serde_json::from_str::<KWinCommand>(&first_json).unwrap(), first);
        assert_eq!(serde_json::from_str::<KWinCommand>(&second_json).unwrap(), second);
        assert_eq!(bridge.next_command_json("script").await.unwrap(), None);
    }

    #[tokio::test]
    async fn complete_command_records_result_by_command_id() {
        let bridge = KWinBridge::new();
        let command = bridge.queue_command("close", json!({ "windowId": "a" })).await;

        bridge.complete_command_json(&command.command_id, r#"{"ok":true}"#).await;

        let completion = bridge.completion(&command.command_id).await.unwrap();
        assert_eq!(completion.command_id, command.command_id);
        assert_eq!(completion.result_json, r#"{"ok":true}"#);
    }
}
