use std::{collections::HashMap, os::fd::OwnedFd as StdOwnedFd};

use futures_util::StreamExt;
use porthole_core::{ErrorCode, PortholeError};
use serde_json::json;
use tokio::time::{Duration, timeout};
use uuid::Uuid;
use zbus::{
    Connection, Proxy,
    proxy::SignalStream,
    zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value},
};

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST_IFACE: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

const RESPONSE_SUCCESS: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const PORTAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

const SOURCE_TYPE_WINDOW: u32 = 2;
const CURSOR_MODE_HIDDEN: u32 = 1;

#[derive(Clone, Debug, Default)]
pub struct ScreenCastPortal;

#[derive(Debug)]
pub struct ScreenCastSession {
    pub _connection: Connection,
    pub session_path: OwnedObjectPath,
    pub pipewire_remote: StdOwnedFd,
    pub streams: Vec<ScreenCastStream>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCastStream {
    pub node_id: u32,
    pub pipewire_serial: Option<u64>,
    pub size: Option<(i32, i32)>,
    pub source_type: Option<u32>,
}

impl ScreenCastPortal {
    pub fn new() -> Self {
        Self
    }

    pub async fn start_window_session(&self) -> Result<ScreenCastSession, PortholeError> {
        let connection = Connection::session()
            .await
            .map_err(|error| portal_unavailable(format!("cannot connect to session bus: {error}")))?;
        let proxy = screencast_proxy(&connection).await?;

        let create_token = token_string("create");
        let (create_handle, mut create_responses) = prepare_response_wait(&connection, &create_token).await?;
        let returned_create_handle: OwnedObjectPath = proxy
            .call(
                "CreateSession",
                &options([
                    ("handle_token", Value::from(create_token)),
                    ("session_handle_token", token_value("session")),
                ]),
            )
            .await
            .map_err(portal_call_failed)?;
        ensure_expected_handle(&create_handle, &returned_create_handle)?;
        let create_results = wait_prepared_response(&mut create_responses, &create_handle).await?;
        let session_handle = string_result(&create_results, "session_handle")?;
        let session_path = OwnedObjectPath::try_from(session_handle.as_str()).map_err(|error| {
            PortholeError::new(
                ErrorCode::InternalError,
                format!("portal returned invalid session_handle {session_handle:?}: {error}"),
            )
        })?;

        let select_token = token_string("select");
        let (select_handle, mut select_responses) = prepare_response_wait(&connection, &select_token).await?;
        let returned_select_handle: OwnedObjectPath = proxy
            .call(
                "SelectSources",
                &(
                    ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                    options([
                        ("handle_token", Value::from(select_token)),
                        ("types", Value::U32(SOURCE_TYPE_WINDOW)),
                        ("multiple", Value::Bool(false)),
                        ("cursor_mode", Value::U32(CURSOR_MODE_HIDDEN)),
                    ]),
                ),
            )
            .await
            .map_err(portal_call_failed)?;
        ensure_expected_handle(&select_handle, &returned_select_handle)?;
        wait_prepared_response(&mut select_responses, &select_handle).await?;

        let start_token = token_string("start");
        let (start_handle, mut start_responses) = prepare_response_wait(&connection, &start_token).await?;
        let returned_start_handle: OwnedObjectPath = proxy
            .call(
                "Start",
                &(
                    ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                    "",
                    options([("handle_token", Value::from(start_token))]),
                ),
            )
            .await
            .map_err(portal_call_failed)?;
        ensure_expected_handle(&start_handle, &returned_start_handle)?;
        let start_results = wait_prepared_response(&mut start_responses, &start_handle).await?;
        let streams = streams_result(&start_results)?;

        let remote: OwnedFd = proxy
            .call(
                "OpenPipeWireRemote",
                &(
                    ObjectPath::try_from(session_path.as_str()).map_err(portal_variant_error)?,
                    empty_options(),
                ),
            )
            .await
            .map_err(portal_call_failed)?;
        let pipewire_remote: StdOwnedFd = remote.into();

        Ok(ScreenCastSession {
            _connection: connection,
            session_path,
            pipewire_remote,
            streams,
        })
    }
}

async fn screencast_proxy(connection: &Connection) -> Result<Proxy<'_>, PortholeError> {
    Proxy::new(connection, PORTAL_DEST, PORTAL_PATH, SCREENCAST_IFACE)
        .await
        .map_err(|error| portal_unavailable(format!("ScreenCast portal is unavailable: {error}")))
}

async fn prepare_response_wait(connection: &Connection, token: &str) -> Result<(OwnedObjectPath, SignalStream<'static>), PortholeError> {
    let handle = request_path(connection, token)?;
    let proxy = Proxy::new(connection, PORTAL_DEST, handle.as_str(), REQUEST_IFACE)
        .await
        .map_err(portal_call_failed)?;
    let responses = proxy.receive_signal("Response").await.map_err(portal_call_failed)?;
    Ok((handle, responses))
}

async fn wait_prepared_response(
    responses: &mut SignalStream<'static>,
    handle: &OwnedObjectPath,
) -> Result<HashMap<String, OwnedValue>, PortholeError> {
    let Some(message) = timeout(PORTAL_RESPONSE_TIMEOUT, responses.next())
        .await
        .map_err(|_| portal_response_timeout(handle.as_str()))?
    else {
        return Err(portal_unavailable("portal request closed before Response signal"));
    };
    let (response, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().map_err(portal_call_failed)?;
    match response {
        RESPONSE_SUCCESS => Ok(results),
        RESPONSE_CANCELLED => Err(portal_cancelled("ScreenCast portal consent was cancelled")),
        other => Err(PortholeError::new(
            ErrorCode::SystemPermissionRequestFailed,
            format!("ScreenCast portal request failed with response code {other}"),
        )
        .with_details(json!({
            "permission": "screencast",
            "reason": format!("portal response code {other}"),
            "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
            "binary_path": current_exe(),
        }))),
    }
}

fn request_path(connection: &Connection, token: &str) -> Result<OwnedObjectPath, PortholeError> {
    let unique_name = connection
        .unique_name()
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, "session bus connection has no unique name"))?;
    let sender = unique_name.as_str().trim_start_matches(':').replace('.', "_");
    OwnedObjectPath::try_from(format!("{PORTAL_PATH}/request/{sender}/{token}")).map_err(portal_variant_error)
}

fn ensure_expected_handle(expected: &OwnedObjectPath, returned: &OwnedObjectPath) -> Result<(), PortholeError> {
    if expected == returned {
        Ok(())
    } else {
        Err(PortholeError::new(
            ErrorCode::InternalError,
            format!("portal returned request handle {returned}, expected {expected}"),
        ))
    }
}

fn streams_result(results: &HashMap<String, OwnedValue>) -> Result<Vec<ScreenCastStream>, PortholeError> {
    let value = results
        .get("streams")
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, "portal response missing streams"))?;
    let streams = Vec::<(u32, HashMap<String, OwnedValue>)>::try_from(value.try_clone().map_err(portal_variant_error)?)
        .map_err(portal_variant_error)?;
    parse_streams(streams)
}

fn parse_streams(streams: Vec<(u32, HashMap<String, OwnedValue>)>) -> Result<Vec<ScreenCastStream>, PortholeError> {
    if streams.is_empty() {
        return Err(PortholeError::new(
            ErrorCode::InternalError,
            "ScreenCast portal returned no streams",
        ));
    }
    streams
        .into_iter()
        .map(|(node_id, properties)| {
            Ok(ScreenCastStream {
                node_id,
                pipewire_serial: optional_u64(&properties, "pipewire-serial"),
                size: optional_i32_pair(&properties, "size")?,
                source_type: optional_u32(&properties, "source_type"),
            })
        })
        .collect()
}

fn options<'a, const N: usize>(entries: [(&'static str, Value<'a>); N]) -> HashMap<&'static str, Value<'a>> {
    entries.into_iter().collect()
}

fn empty_options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

fn token_value(prefix: &str) -> Value<'static> {
    Value::from(token_string(prefix))
}

fn token_string(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn string_result(results: &HashMap<String, OwnedValue>, key: &str) -> Result<String, PortholeError> {
    let value = results
        .get(key)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, format!("portal response missing {key}")))?;
    String::try_from(value.try_clone().map_err(portal_variant_error)?).map_err(portal_variant_error)
}

fn optional_u32(values: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    values
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| u32::try_from(value).ok())
}

fn optional_u64(values: &HashMap<String, OwnedValue>, key: &str) -> Option<u64> {
    values
        .get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| u64::try_from(value).ok())
}

fn optional_i32_pair(values: &HashMap<String, OwnedValue>, key: &str) -> Result<Option<(i32, i32)>, PortholeError> {
    values
        .get(key)
        .map(|value| <(i32, i32)>::try_from(value.try_clone().map_err(portal_variant_error)?).map_err(portal_variant_error))
        .transpose()
}

pub fn permission_needed(reason: impl Into<String>) -> PortholeError {
    PortholeError::new(ErrorCode::SystemPermissionNeeded, reason.into()).with_details(json!({
        "permission": "screencast",
        "remediation": {
            "cli_command": "porthole onboard",
            "requires_daemon_restart": false,
            "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
            "binary_path": current_exe(),
        }
    }))
}

fn portal_unavailable(reason: impl Into<String>) -> PortholeError {
    PortholeError::new(ErrorCode::SystemPermissionRequestFailed, reason.into()).with_details(json!({
        "permission": "screencast",
        "reason": "ScreenCast portal is unavailable",
        "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
        "binary_path": current_exe(),
    }))
}

fn portal_cancelled(reason: impl Into<String>) -> PortholeError {
    PortholeError::new(ErrorCode::SystemPermissionRequestFailed, reason.into()).with_details(json!({
        "permission": "screencast",
        "reason": "portal consent cancelled",
        "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
        "binary_path": current_exe(),
    }))
}

fn portal_call_failed(error: impl std::fmt::Display) -> PortholeError {
    PortholeError::new(
        ErrorCode::SystemPermissionRequestFailed,
        format!("ScreenCast portal call failed: {error}"),
    )
}

fn portal_variant_error(error: impl std::fmt::Display) -> PortholeError {
    PortholeError::new(ErrorCode::InternalError, format!("ScreenCast portal value error: {error}"))
}

fn portal_response_timeout(handle: &str) -> PortholeError {
    PortholeError::new(
        ErrorCode::SystemPermissionRequestFailed,
        format!("ScreenCast portal request {handle} timed out waiting for Response signal"),
    )
    .with_details(json!({
        "permission": "screencast",
        "reason": "portal response timeout",
        "settings_path": "KDE System Settings -> Security & Privacy -> Application Permissions",
        "binary_path": current_exe(),
    }))
}

fn current_exe() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_screencast_stream_metadata() {
        let mut properties = HashMap::new();
        properties.insert("pipewire-serial".to_string(), OwnedValue::from(99_u64));
        properties.insert("source_type".to_string(), OwnedValue::from(SOURCE_TYPE_WINDOW));
        let streams = vec![(7_u32, properties)];

        let parsed = parse_streams(streams).unwrap();
        assert_eq!(
            parsed,
            vec![ScreenCastStream {
                node_id: 7,
                pipewire_serial: Some(99),
                size: None,
                source_type: Some(SOURCE_TYPE_WINDOW),
            }]
        );
    }

    #[test]
    fn rejects_empty_screencast_streams() {
        assert!(parse_streams(Vec::new()).is_err());
    }
}
