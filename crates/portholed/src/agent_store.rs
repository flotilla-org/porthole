use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use porthole_core::agent_policy::{
    ActionClass, AgentId, Constraint, Denial, DenialId, DurationSpec, Grant, GrantId, PermissionRequestId, PolicySnapshot, TargetSelector,
};
use porthole_protocol::agent_permissions::AgentIdentityMetadata;
use rusqlite::{Connection, OptionalExtension, params};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AgentStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("blocking database task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("agent policy store lock poisoned")]
    LockPoisoned,
    #[error("permission request not found")]
    PermissionRequestNotFound,
    #[error("permission request is not pending")]
    PermissionRequestNotPending,
    #[error("agent identity not found")]
    IdentityNotFound,
    #[error("unknown permission request status {0}")]
    UnknownPermissionRequestStatus(String),
}

pub type StoreResult<T> = Result<T, AgentStoreError>;

#[derive(Clone)]
pub struct AgentPolicyStore {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedAgentIdentity {
    pub agent_id: AgentId,
    pub token_id: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedAgentToken {
    pub token_id: String,
    pub agent_id: AgentId,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAgentIdentity {
    pub agent_id: AgentId,
    pub display_name: String,
    pub metadata: Option<AgentIdentityMetadata>,
    pub created_at_unix_ms: u64,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAgentToken {
    pub token_id: String,
    pub agent_id: AgentId,
    pub created_at_unix_ms: u64,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionRequestStatus {
    Pending,
    Approved,
    Denied,
}

impl PermissionRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }

    fn parse(s: &str) -> StoreResult<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            other => Err(AgentStoreError::UnknownPermissionRequestStatus(other.to_string())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPermissionRequest {
    pub request_id: PermissionRequestId,
    pub agent_id: AgentId,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
    pub reason: Option<String>,
    pub status: PermissionRequestStatus,
    pub created_at_unix_ms: u64,
    pub resolved_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredGrant {
    pub grant_id: GrantId,
    pub agent_id: AgentId,
    pub origin_request_id: Option<PermissionRequestId>,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
    pub duration: DurationSpec,
    pub constraints: Vec<Constraint>,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: Option<u64>,
    pub consumed_at_unix_ms: Option<u64>,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionAuditRecord {
    pub agent_id: AgentId,
    pub request_id: Option<PermissionRequestId>,
    pub route: String,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
    pub grant_id: Option<GrantId>,
    pub denial_id: Option<DenialId>,
    pub executed_at_unix_ms: u64,
}

impl AgentPolicyStore {
    pub fn open_in_memory_sync() -> StoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn open_in_memory() -> StoreResult<Self> {
        Self::open_in_memory_sync()
    }

    pub async fn open_default() -> StoreResult<Self> {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            AgentStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME is not set; cannot locate Application Support",
            ))
        })?;
        Self::open_at_path(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Porthole")
                .join("agent-policy.sqlite"),
        )
        .await
    }

    pub async fn open_at_path(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            set_owner_only_dir_permissions(parent)?;
        }
        let conn = Connection::open(path)?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    async fn with_conn<T, F>(&self, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> StoreResult<T> + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().map_err(|_| AgentStoreError::LockPoisoned)?;
            f(&mut guard)
        })
        .await?
    }

    pub async fn create_identity(
        &self,
        display_name: impl Into<String>,
        metadata: Option<AgentIdentityMetadata>,
        created_at_unix_ms: u64,
    ) -> StoreResult<CreatedAgentIdentity> {
        let display_name = display_name.into();
        self.with_conn(move |conn| {
            let agent_id = AgentId::from(format!("agent_{}", Uuid::new_v4().simple()));
            let token_id = format!("tok_{}", Uuid::new_v4().simple());
            let secret = Uuid::new_v4().simple().to_string();
            let token = format!("pta_{}.{secret}", agent_id.as_str());
            let token_hash = hash_token(&token);
            let metadata_json = serde_json::to_string(&metadata)?;
            conn.execute(
                "INSERT INTO agent_identities(agent_id, display_name, metadata_json, created_at_unix_ms, revoked_at_unix_ms)
                 VALUES(?1, ?2, ?3, ?4, NULL)",
                params![agent_id.as_str(), display_name, metadata_json, created_at_unix_ms],
            )?;
            conn.execute(
                "INSERT INTO agent_tokens(token_id, agent_id, token_hash, created_at_unix_ms, revoked_at_unix_ms)
                 VALUES(?1, ?2, ?3, ?4, NULL)",
                params![token_id, agent_id.as_str(), token_hash, created_at_unix_ms],
            )?;
            Ok(CreatedAgentIdentity { agent_id, token_id, token })
        })
        .await
    }

    pub async fn list_identities(&self) -> StoreResult<Vec<StoredAgentIdentity>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT agent_id, display_name, metadata_json, created_at_unix_ms, revoked_at_unix_ms
                 FROM agent_identities
                 ORDER BY created_at_unix_ms, agent_id",
            )?;
            let rows = stmt.query_map([], identity_tuple_from_row)?;
            rows.map(|row| stored_identity_from_tuple(row?)).collect()
        })
        .await
    }

    pub async fn get_identity(&self, agent_id: &AgentId) -> StoreResult<Option<StoredAgentIdentity>> {
        let agent_id = agent_id.clone();
        self.with_conn(move |conn| select_identity(conn, &agent_id)).await
    }

    pub async fn revoke_identity(&self, agent_id: &AgentId, revoked_at_unix_ms: u64) -> StoreResult<bool> {
        let agent_id = agent_id.clone();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let updated = tx.execute(
                "UPDATE agent_identities
                 SET revoked_at_unix_ms = ?1
                 WHERE agent_id = ?2
                   AND revoked_at_unix_ms IS NULL",
                params![revoked_at_unix_ms, agent_id.as_str()],
            )?;
            tx.execute(
                "UPDATE agent_tokens
                 SET revoked_at_unix_ms = ?1
                 WHERE agent_id = ?2
                   AND revoked_at_unix_ms IS NULL",
                params![revoked_at_unix_ms, agent_id.as_str()],
            )?;
            tx.commit()?;
            Ok(updated == 1)
        })
        .await
    }

    pub async fn mint_token(&self, agent_id: &AgentId, created_at_unix_ms: u64) -> StoreResult<CreatedAgentToken> {
        let agent_id = agent_id.clone();
        self.with_conn(move |conn| {
            let exists = conn
                .query_row(
                    "SELECT 1 FROM agent_identities WHERE agent_id = ?1 AND revoked_at_unix_ms IS NULL",
                    params![agent_id.as_str()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Err(AgentStoreError::IdentityNotFound);
            }
            let token_id = format!("tok_{}", Uuid::new_v4().simple());
            let secret = Uuid::new_v4().simple().to_string();
            let token = format!("pta_{}.{secret}", agent_id.as_str());
            conn.execute(
                "INSERT INTO agent_tokens(token_id, agent_id, token_hash, created_at_unix_ms, revoked_at_unix_ms)
                 VALUES(?1, ?2, ?3, ?4, NULL)",
                params![token_id, agent_id.as_str(), hash_token(&token), created_at_unix_ms],
            )?;
            Ok(CreatedAgentToken { token_id, agent_id, token })
        })
        .await
    }

    pub async fn revoke_token(&self, agent_id: &AgentId, token_id: &str, revoked_at_unix_ms: u64) -> StoreResult<bool> {
        let agent_id = agent_id.clone();
        let token_id = token_id.to_string();
        self.with_conn(move |conn| {
            let updated = conn.execute(
                "UPDATE agent_tokens
                 SET revoked_at_unix_ms = ?1
                 WHERE agent_id = ?2
                   AND token_id = ?3
                   AND revoked_at_unix_ms IS NULL",
                params![revoked_at_unix_ms, agent_id.as_str(), token_id],
            )?;
            Ok(updated == 1)
        })
        .await
    }

    pub async fn authenticate_agent_token(&self, token: &str) -> StoreResult<Option<AgentId>> {
        let token = token.to_string();
        self.with_conn(move |conn| {
            let Some(agent_id) = parse_token_agent_id(&token) else {
                return Ok(None);
            };
            let candidate_hash = hash_token(&token);
            let mut stmt = conn.prepare(
                "SELECT t.token_hash
                 FROM agent_tokens t
                 INNER JOIN agent_identities i ON i.agent_id = t.agent_id
                 WHERE t.agent_id = ?1
                   AND t.revoked_at_unix_ms IS NULL
                   AND i.revoked_at_unix_ms IS NULL",
            )?;
            let rows = stmt.query_map(params![agent_id.as_str()], |row| row.get::<_, String>(0))?;
            for row in rows {
                let stored_hash = row?;
                if bool::from(candidate_hash.as_bytes().ct_eq(stored_hash.as_bytes())) {
                    return Ok(Some(agent_id));
                }
            }
            Ok(None)
        })
        .await
    }

    pub async fn create_pending_request(
        &self,
        agent_id: AgentId,
        target: TargetSelector,
        actions: Vec<ActionClass>,
        reason: Option<String>,
        created_at_unix_ms: u64,
    ) -> StoreResult<StoredPermissionRequest> {
        self.with_conn(move |conn| {
            let actions = canonicalize_actions(actions);
            let target_json = serde_json::to_string(&target)?;
            let actions_json = serde_json::to_string(&actions)?;
            if let Some(existing) = select_pending_request(conn, &agent_id, &target_json, &actions_json)? {
                return Ok(existing);
            }

            let request_id = PermissionRequestId::from(format!("apr_{}", Uuid::new_v4().simple()));
            let stored_reason = reason.clone();
            conn.execute(
                "INSERT INTO agent_permission_requests(request_id, agent_id, target_json, actions_json, reason, status, created_at_unix_ms, resolved_at_unix_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, 'pending', ?6, NULL)",
                params![
                    request_id.as_str(),
                    agent_id.as_str(),
                    target_json,
                    actions_json,
                    reason,
                    created_at_unix_ms,
                ],
            )?;
            insert_audit(
                conn,
                &agent_id,
                Some(&request_id),
                None,
                &target,
                &actions,
                "requested",
                None,
                None,
                Some(created_at_unix_ms),
                None,
                None,
                None,
                stored_reason.as_deref(),
            )?;
            Ok(StoredPermissionRequest {
                request_id,
                agent_id,
                target,
                actions,
                reason: stored_reason,
                status: PermissionRequestStatus::Pending,
                created_at_unix_ms,
                resolved_at_unix_ms: None,
            })
        })
        .await
    }

    pub async fn get_permission_request(&self, request_id: &PermissionRequestId) -> StoreResult<Option<StoredPermissionRequest>> {
        let request_id = request_id.clone();
        self.with_conn(move |conn| select_request_by_id(conn, &request_id)).await
    }

    pub async fn approve_request(
        &self,
        request_id: &PermissionRequestId,
        duration: DurationSpec,
        constraints: Vec<Constraint>,
        decided_at_unix_ms: u64,
    ) -> StoreResult<StoredGrant> {
        let request_id = request_id.clone();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let request = select_request_by_id_tx(&tx, &request_id)?.ok_or(AgentStoreError::PermissionRequestNotFound)?;
            if request.status != PermissionRequestStatus::Pending {
                return Err(AgentStoreError::PermissionRequestNotPending);
            }

            let grant_id = GrantId::from(format!("grant_{}", Uuid::new_v4().simple()));
            let actions = canonicalize_actions(request.actions.clone());
            let target_json = serde_json::to_string(&request.target)?;
            let actions_json = serde_json::to_string(&actions)?;
            let duration_json = serde_json::to_string(&duration)?;
            let constraints_json = serde_json::to_string(&constraints)?;
            let expires_at_unix_ms = match duration {
                DurationSpec::TimeBounded { expires_at_unix_ms } => Some(expires_at_unix_ms),
                _ => None,
            };
            tx.execute(
                "INSERT INTO agent_grants(grant_id, agent_id, origin_request_id, target_json, actions_json, duration_json, constraints_json, created_at_unix_ms, expires_at_unix_ms, consumed_at_unix_ms, revoked_at_unix_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL)",
                params![
                    grant_id.as_str(),
                    request.agent_id.as_str(),
                    request.request_id.as_str(),
                    target_json,
                    actions_json,
                    duration_json,
                    constraints_json,
                    decided_at_unix_ms,
                    expires_at_unix_ms,
                ],
            )?;
            tx.execute(
                "UPDATE agent_permission_requests
                 SET status = 'approved', resolved_at_unix_ms = ?1
                 WHERE request_id = ?2",
                params![decided_at_unix_ms, request.request_id.as_str()],
            )?;
            insert_audit(
                &tx,
                &request.agent_id,
                Some(&request.request_id),
                None,
                &request.target,
                &actions,
                "approved",
                Some(&grant_id),
                None,
                None,
                Some(decided_at_unix_ms),
                expires_at_unix_ms,
                None,
                None,
            )?;
            tx.commit()?;
            Ok(StoredGrant {
                grant_id,
                agent_id: request.agent_id,
                origin_request_id: Some(request.request_id),
                target: request.target,
                actions,
                duration,
                constraints,
                created_at_unix_ms: decided_at_unix_ms,
                expires_at_unix_ms,
                consumed_at_unix_ms: None,
                revoked_at_unix_ms: None,
            })
        })
        .await
    }

    pub async fn list_permission_requests(&self) -> StoreResult<Vec<StoredPermissionRequest>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT request_id, agent_id, target_json, actions_json, reason, status, created_at_unix_ms, resolved_at_unix_ms
                 FROM agent_permission_requests
                 WHERE status = 'pending'
                 ORDER BY created_at_unix_ms, request_id",
            )?;
            let rows = stmt.query_map([], request_tuple_from_row)?;
            rows.map(|row| stored_request_from_tuple(row?)).collect()
        })
        .await
    }

    pub async fn list_active_grants(&self, now_unix_ms: u64) -> StoreResult<Vec<StoredGrant>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT grant_id, agent_id, origin_request_id, target_json, actions_json, duration_json, constraints_json, created_at_unix_ms, expires_at_unix_ms, consumed_at_unix_ms, revoked_at_unix_ms
                 FROM agent_grants
                 WHERE revoked_at_unix_ms IS NULL
                   AND consumed_at_unix_ms IS NULL
                   AND (expires_at_unix_ms IS NULL OR expires_at_unix_ms > ?1)
                 ORDER BY created_at_unix_ms, grant_id",
            )?;
            let rows = stmt.query_map(params![now_unix_ms], grant_tuple_from_row)?;
            rows.map(|row| stored_grant_from_tuple(row?)).collect()
        })
        .await
    }

    pub async fn revoke_grant(&self, grant_id: &GrantId, revoked_at_unix_ms: u64) -> StoreResult<bool> {
        let grant_id = grant_id.clone();
        self.with_conn(move |conn| {
            let updated = conn.execute(
                "UPDATE agent_grants
                 SET revoked_at_unix_ms = ?1
                 WHERE grant_id = ?2
                   AND revoked_at_unix_ms IS NULL",
                params![revoked_at_unix_ms, grant_id.as_str()],
            )?;
            Ok(updated == 1)
        })
        .await
    }

    pub async fn deny_request(
        &self,
        request_id: &PermissionRequestId,
        remember: bool,
        reason: Option<String>,
        decided_at_unix_ms: u64,
    ) -> StoreResult<Option<Denial>> {
        let request_id = request_id.clone();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let request = select_request_by_id_tx(&tx, &request_id)?.ok_or(AgentStoreError::PermissionRequestNotFound)?;
            if request.status != PermissionRequestStatus::Pending {
                return Err(AgentStoreError::PermissionRequestNotPending);
            }

            let actions = canonicalize_actions(request.actions.clone());
            let denial = if remember {
                let denial_id = DenialId::from(format!("deny_{}", Uuid::new_v4().simple()));
                tx.execute(
                    "INSERT INTO agent_denials(denial_id, agent_id, target_json, actions_json, reason, created_at_unix_ms, expires_at_unix_ms)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                    params![
                        denial_id.as_str(),
                        request.agent_id.as_str(),
                        serde_json::to_string(&request.target)?,
                        serde_json::to_string(&actions)?,
                        reason.as_deref(),
                        decided_at_unix_ms,
                    ],
                )?;
                Some(Denial {
                    denial_id,
                    agent_id: request.agent_id.clone(),
                    target: request.target.clone(),
                    actions: actions.clone(),
                    expires_at_unix_ms: None,
                })
            } else {
                None
            };
            tx.execute(
                "UPDATE agent_permission_requests
                 SET status = 'denied', resolved_at_unix_ms = ?1
                 WHERE request_id = ?2",
                params![decided_at_unix_ms, request.request_id.as_str()],
            )?;
            insert_audit(
                &tx,
                &request.agent_id,
                Some(&request.request_id),
                None,
                &request.target,
                &actions,
                "denied",
                None,
                denial.as_ref().map(|d| &d.denial_id),
                None,
                Some(decided_at_unix_ms),
                None,
                None,
                reason.as_deref(),
            )?;
            tx.commit()?;
            Ok(denial)
        })
        .await
    }

    pub async fn load_policy_snapshot(&self, agent_id: &AgentId) -> StoreResult<PolicySnapshot> {
        let agent_id = agent_id.clone();
        self.with_conn(move |conn| {
            let mut grants = Vec::new();
            let mut consumed_once_grants = Vec::new();
            let mut grant_stmt = conn.prepare(
                "SELECT grant_id, target_json, actions_json, duration_json, constraints_json, consumed_at_unix_ms
                 FROM agent_grants
                 WHERE agent_id = ?1
                   AND revoked_at_unix_ms IS NULL",
            )?;
            let grant_rows = grant_stmt.query_map(params![agent_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<u64>>(5)?,
                ))
            })?;
            for row in grant_rows {
                let (grant_id, target_json, actions_json, duration_json, constraints_json, consumed_at_unix_ms) = row?;
                let grant_id = GrantId::from(grant_id);
                let duration: DurationSpec = from_json(&duration_json)?;
                if duration == DurationSpec::Once && consumed_at_unix_ms.is_some() {
                    consumed_once_grants.push(grant_id.clone());
                }
                grants.push(Grant {
                    grant_id,
                    agent_id: agent_id.clone(),
                    target: from_json(&target_json)?,
                    actions: from_json(&actions_json)?,
                    duration,
                    constraints: from_json(&constraints_json)?,
                });
            }

            let mut denials = Vec::new();
            let mut denial_stmt = conn.prepare(
                "SELECT denial_id, target_json, actions_json, expires_at_unix_ms
                 FROM agent_denials
                 WHERE agent_id = ?1",
            )?;
            let denial_rows = denial_stmt.query_map(params![agent_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<u64>>(3)?,
                ))
            })?;
            for row in denial_rows {
                let (denial_id, target_json, actions_json, expires_at_unix_ms) = row?;
                denials.push(Denial {
                    denial_id: DenialId::from(denial_id),
                    agent_id: agent_id.clone(),
                    target: from_json(&target_json)?,
                    actions: from_json(&actions_json)?,
                    expires_at_unix_ms,
                });
            }

            Ok(PolicySnapshot {
                grants,
                denials,
                consumed_once_grants,
            })
        })
        .await
    }

    pub async fn try_consume_once_grant(&self, grant_id: &GrantId, consumed_at_unix_ms: u64) -> StoreResult<bool> {
        let grant_id = grant_id.clone();
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let existing = tx
                .query_row(
                    "SELECT duration_json, consumed_at_unix_ms, revoked_at_unix_ms
                     FROM agent_grants
                     WHERE grant_id = ?1",
                    params![grant_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<u64>>(1)?,
                            row.get::<_, Option<u64>>(2)?,
                        ))
                    },
                )
                .optional()?;
            let Some((duration_json, consumed_at, revoked_at)) = existing else {
                return Ok(false);
            };
            let duration: DurationSpec = from_json(&duration_json)?;
            if duration != DurationSpec::Once || consumed_at.is_some() || revoked_at.is_some() {
                return Ok(false);
            }
            let updated = tx.execute(
                "UPDATE agent_grants
                 SET consumed_at_unix_ms = ?1
                 WHERE grant_id = ?2
                   AND consumed_at_unix_ms IS NULL
                   AND revoked_at_unix_ms IS NULL",
                params![consumed_at_unix_ms, grant_id.as_str()],
            )?;
            tx.commit()?;
            Ok(updated == 1)
        })
        .await
    }

    pub async fn insert_execution_audit(&self, record: ExecutionAuditRecord) -> StoreResult<()> {
        self.with_conn(move |conn| {
            let actions = canonicalize_actions(record.actions);
            insert_audit(
                conn,
                &record.agent_id,
                record.request_id.as_ref(),
                Some(&record.route),
                &record.target,
                &actions,
                "executed",
                record.grant_id.as_ref(),
                record.denial_id.as_ref(),
                None,
                None,
                None,
                Some(record.executed_at_unix_ms),
                None,
            )
        })
        .await
    }

    #[cfg(test)]
    async fn debug_table_names(&self) -> StoreResult<Vec<String>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_plaintext_token_count(&self) -> StoreResult<u64> {
        self.with_conn(move |conn| {
            conn.query_row("SELECT COUNT(*) FROM agent_tokens WHERE token_hash LIKE 'pta_%'", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_token_hashes_for_agent(&self, agent_id: &AgentId) -> StoreResult<Vec<String>> {
        let agent_id = agent_id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT token_hash FROM agent_tokens WHERE agent_id = ?1 ORDER BY token_id")?;
            let rows = stmt.query_map(params![agent_id.as_str()], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_permission_request_count(&self) -> StoreResult<u64> {
        self.with_conn(move |conn| {
            conn.query_row("SELECT COUNT(*) FROM agent_permission_requests", [], |row| row.get::<_, u64>(0))
                .map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_audit_decisions(&self) -> StoreResult<Vec<String>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT decision FROM agent_permission_audit ORDER BY rowid")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_denial_reasons(&self) -> StoreResult<Vec<Option<String>>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT reason FROM agent_denials ORDER BY rowid")?;
            let rows = stmt.query_map([], |row| row.get::<_, Option<String>>(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_audit_reasons(&self) -> StoreResult<Vec<Option<String>>> {
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare("SELECT reason FROM agent_permission_audit ORDER BY rowid")?;
            let rows = stmt.query_map([], |row| row.get::<_, Option<String>>(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
        .await
    }

    #[cfg(test)]
    async fn debug_insert_token_hash_for_agent(&self, agent_id: &AgentId, token: &str, created_at_unix_ms: u64) -> StoreResult<()> {
        let agent_id = agent_id.clone();
        let token = token.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO agent_tokens(token_id, agent_id, token_hash, created_at_unix_ms, revoked_at_unix_ms)
                 VALUES(?1, ?2, ?3, ?4, NULL)",
                params![
                    format!("tok_{}", Uuid::new_v4().simple()),
                    agent_id.as_str(),
                    hash_token(&token),
                    created_at_unix_ms,
                ],
            )?;
            Ok(())
        })
        .await
    }
}

fn init_schema(conn: &Connection) -> StoreResult<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS agent_identities(
            agent_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            metadata_json TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            revoked_at_unix_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_tokens(
            token_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            token_hash TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            revoked_at_unix_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_grants(
            grant_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            origin_request_id TEXT,
            target_json TEXT NOT NULL,
            actions_json TEXT NOT NULL,
            duration_json TEXT NOT NULL,
            constraints_json TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            expires_at_unix_ms INTEGER,
            consumed_at_unix_ms INTEGER,
            revoked_at_unix_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_denials(
            denial_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            target_json TEXT NOT NULL,
            actions_json TEXT NOT NULL,
            reason TEXT,
            created_at_unix_ms INTEGER NOT NULL,
            expires_at_unix_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_permission_requests(
            request_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            target_json TEXT NOT NULL,
            actions_json TEXT NOT NULL,
            reason TEXT,
            status TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            resolved_at_unix_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS agent_permission_audit(
            audit_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            request_id TEXT,
            route TEXT,
            target_json TEXT NOT NULL,
            actions_json TEXT NOT NULL,
            decision TEXT NOT NULL,
            grant_id TEXT,
            denial_id TEXT,
            reason TEXT,
            requested_at_unix_ms INTEGER,
            decided_at_unix_ms INTEGER,
            expires_at_unix_ms INTEGER,
            executed_at_unix_ms INTEGER
        );
        CREATE INDEX IF NOT EXISTS agent_tokens_agent_id ON agent_tokens(agent_id);
        CREATE UNIQUE INDEX IF NOT EXISTS agent_permission_requests_pending_unique
            ON agent_permission_requests(agent_id, target_json, actions_json)
            WHERE status = 'pending';
        CREATE UNIQUE INDEX IF NOT EXISTS agent_grants_origin_request_unique
            ON agent_grants(origin_request_id)
            WHERE origin_request_id IS NOT NULL;
        ",
    )?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_dir_permissions(path: &Path) -> StoreResult<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_dir_permissions(_path: &Path) -> StoreResult<()> {
    Ok(())
}

fn select_pending_request(
    conn: &Connection,
    agent_id: &AgentId,
    target_json: &str,
    actions_json: &str,
) -> StoreResult<Option<StoredPermissionRequest>> {
    let row = conn
        .query_row(
            "SELECT request_id, agent_id, target_json, actions_json, reason, status, created_at_unix_ms, resolved_at_unix_ms
             FROM agent_permission_requests
             WHERE agent_id = ?1
               AND target_json = ?2
               AND actions_json = ?3
               AND status = 'pending'",
            params![agent_id.as_str(), target_json, actions_json],
            request_tuple_from_row,
        )
        .optional()?;
    row.map(stored_request_from_tuple).transpose()
}

fn select_request_by_id(conn: &Connection, request_id: &PermissionRequestId) -> StoreResult<Option<StoredPermissionRequest>> {
    let row = conn
        .query_row(
            "SELECT request_id, agent_id, target_json, actions_json, reason, status, created_at_unix_ms, resolved_at_unix_ms
             FROM agent_permission_requests
             WHERE request_id = ?1",
            params![request_id.as_str()],
            request_tuple_from_row,
        )
        .optional()?;
    row.map(stored_request_from_tuple).transpose()
}

fn select_request_by_id_tx(
    conn: &rusqlite::Transaction<'_>,
    request_id: &PermissionRequestId,
) -> StoreResult<Option<StoredPermissionRequest>> {
    let row = conn
        .query_row(
            "SELECT request_id, agent_id, target_json, actions_json, reason, status, created_at_unix_ms, resolved_at_unix_ms
             FROM agent_permission_requests
             WHERE request_id = ?1",
            params![request_id.as_str()],
            request_tuple_from_row,
        )
        .optional()?;
    row.map(stored_request_from_tuple).transpose()
}

type RequestTuple = (String, String, String, String, Option<String>, String, u64, Option<u64>);
type IdentityTuple = (String, String, String, u64, Option<u64>);
type GrantTuple = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    u64,
    Option<u64>,
    Option<u64>,
    Option<u64>,
);

fn identity_tuple_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IdentityTuple> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
}

fn stored_identity_from_tuple(row: IdentityTuple) -> StoreResult<StoredAgentIdentity> {
    let (agent_id, display_name, metadata_json, created_at_unix_ms, revoked_at_unix_ms) = row;
    Ok(StoredAgentIdentity {
        agent_id: AgentId::from(agent_id),
        display_name,
        metadata: from_json(&metadata_json)?,
        created_at_unix_ms,
        revoked_at_unix_ms,
    })
}

fn select_identity(conn: &Connection, agent_id: &AgentId) -> StoreResult<Option<StoredAgentIdentity>> {
    let row = conn
        .query_row(
            "SELECT agent_id, display_name, metadata_json, created_at_unix_ms, revoked_at_unix_ms
             FROM agent_identities
             WHERE agent_id = ?1",
            params![agent_id.as_str()],
            identity_tuple_from_row,
        )
        .optional()?;
    row.map(stored_identity_from_tuple).transpose()
}

fn grant_tuple_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GrantTuple> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ))
}

fn stored_grant_from_tuple(row: GrantTuple) -> StoreResult<StoredGrant> {
    let (
        grant_id,
        agent_id,
        origin_request_id,
        target_json,
        actions_json,
        duration_json,
        constraints_json,
        created_at_unix_ms,
        expires_at_unix_ms,
        consumed_at_unix_ms,
        revoked_at_unix_ms,
    ) = row;
    Ok(StoredGrant {
        grant_id: GrantId::from(grant_id),
        agent_id: AgentId::from(agent_id),
        origin_request_id: origin_request_id.map(PermissionRequestId::from),
        target: from_json(&target_json)?,
        actions: from_json(&actions_json)?,
        duration: from_json(&duration_json)?,
        constraints: from_json(&constraints_json)?,
        created_at_unix_ms,
        expires_at_unix_ms,
        consumed_at_unix_ms,
        revoked_at_unix_ms,
    })
}

fn request_tuple_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestTuple> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn stored_request_from_tuple(row: RequestTuple) -> StoreResult<StoredPermissionRequest> {
    let (request_id, agent_id, target_json, actions_json, reason, status, created_at_unix_ms, resolved_at_unix_ms) = row;
    Ok(StoredPermissionRequest {
        request_id: PermissionRequestId::from(request_id),
        agent_id: AgentId::from(agent_id),
        target: from_json(&target_json)?,
        actions: from_json(&actions_json)?,
        reason,
        status: PermissionRequestStatus::parse(&status)?,
        created_at_unix_ms,
        resolved_at_unix_ms,
    })
}

#[allow(clippy::too_many_arguments)]
fn insert_audit(
    conn: &Connection,
    agent_id: &AgentId,
    request_id: Option<&PermissionRequestId>,
    route: Option<&str>,
    target: &TargetSelector,
    actions: &[ActionClass],
    decision: &str,
    grant_id: Option<&GrantId>,
    denial_id: Option<&DenialId>,
    requested_at_unix_ms: Option<u64>,
    decided_at_unix_ms: Option<u64>,
    expires_at_unix_ms: Option<u64>,
    executed_at_unix_ms: Option<u64>,
    reason: Option<&str>,
) -> StoreResult<()> {
    let audit_id = format!("audit_{}", Uuid::new_v4().simple());
    let target_json = serde_json::to_string(target)?;
    let actions_json = serde_json::to_string(&canonicalize_actions(actions.to_vec()))?;
    conn.execute(
        "INSERT INTO agent_permission_audit(audit_id, agent_id, request_id, route, target_json, actions_json, decision, grant_id, denial_id, reason, requested_at_unix_ms, decided_at_unix_ms, expires_at_unix_ms, executed_at_unix_ms)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            audit_id,
            agent_id.as_str(),
            request_id.map(PermissionRequestId::as_str),
            route,
            target_json,
            actions_json,
            decision,
            grant_id.map(GrantId::as_str),
            denial_id.map(DenialId::as_str),
            reason,
            requested_at_unix_ms,
            decided_at_unix_ms,
            expires_at_unix_ms,
            executed_at_unix_ms,
        ],
    )?;
    Ok(())
}

fn canonicalize_actions(mut actions: Vec<ActionClass>) -> Vec<ActionClass> {
    actions.sort();
    actions.dedup();
    actions
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn parse_token_agent_id(token: &str) -> Option<AgentId> {
    let rest = token.strip_prefix("pta_")?;
    let (agent_id, secret) = rest.split_once('.')?;
    if agent_id.is_empty() || secret.is_empty() {
        return None;
    }
    Some(AgentId::from(agent_id))
}

fn from_json<T: DeserializeOwned>(json: &str) -> StoreResult<T> {
    serde_json::from_str(json).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use porthole_core::{
        SurfaceId,
        agent_policy::{
            ActionClass, AgentContext, AuthorizationDecision, Constraint, DurationSpec, GrantId, TargetContext, TargetSelector,
        },
    };
    use porthole_protocol::agent_permissions::AgentIdentityMetadata;

    use super::*;

    const NOW: u64 = 1_000;

    fn target(surface_id: &str) -> TargetSelector {
        TargetSelector::Surface {
            surface_id: SurfaceId::from(surface_id),
        }
    }

    fn target_context(surface_id: &str) -> TargetContext {
        TargetContext {
            surface_id: Some(SurfaceId::from(surface_id)),
            app_bundle_id: None,
            executable_path: None,
            app_name: None,
            launched_by_agent: None,
            frontmost_surface_id: None,
            surface_alive: true,
        }
    }

    async fn pending_request(store: &AgentPolicyStore) -> StoredPermissionRequest {
        let identity = store.create_identity("agent", None, NOW).await.unwrap();
        store
            .create_pending_request(
                identity.agent_id,
                target("surf_1"),
                vec![ActionClass::Drive],
                Some("typing".into()),
                NOW + 1,
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn schema_initializes_in_temp_db() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();

        let tables = store.debug_table_names().await.unwrap();

        for table in [
            "agent_identities",
            "agent_tokens",
            "agent_grants",
            "agent_denials",
            "agent_permission_requests",
            "agent_permission_audit",
        ] {
            assert!(tables.contains(&table.to_string()), "missing table {table}");
        }
    }

    #[tokio::test]
    async fn file_backed_store_initializes_under_explicit_path() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("Application Support").join("Porthole").join("agent-policy.sqlite");

        let store = AgentPolicyStore::open_at_path(&db_path).await.unwrap();
        let tables = store.debug_table_names().await.unwrap();

        assert!(db_path.exists());
        assert!(tables.contains(&"agent_permission_requests".to_string()));
    }

    #[tokio::test]
    async fn creating_identity_returns_agent_id_and_one_plaintext_token_only_once() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();

        let created = store
            .create_identity(
                "Build Agent",
                Some(AgentIdentityMetadata {
                    bundle_id: Some("com.example.agent".into()),
                    executable_path: None,
                    vendor: Some("Example".into()),
                }),
                NOW,
            )
            .await
            .unwrap();

        assert!(created.agent_id.as_str().starts_with("agent_"));
        assert!(created.token_id.starts_with("tok_"));
        assert!(created.token.starts_with(&format!("pta_{}.", created.agent_id)));
        assert_eq!(store.debug_plaintext_token_count().await.unwrap(), 0);
        assert_eq!(store.debug_token_hashes_for_agent(&created.agent_id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn authenticating_with_plaintext_token_returns_agent_id() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let created = store.create_identity("agent", None, NOW).await.unwrap();

        let authenticated = store.authenticate_agent_token(&created.token).await.unwrap();

        assert_eq!(authenticated, Some(created.agent_id));
    }

    #[tokio::test]
    async fn token_hashes_not_plaintext_tokens_are_stored() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let created = store.create_identity("agent", None, NOW).await.unwrap();

        let hashes = store.debug_token_hashes_for_agent(&created.agent_id).await.unwrap();

        assert_eq!(hashes.len(), 1);
        assert_ne!(hashes[0], created.token);
        assert_eq!(hashes[0].len(), 64);
    }

    #[tokio::test]
    async fn creating_pending_request_is_idempotent_for_same_scope_while_pending() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let identity = store.create_identity("agent", None, NOW).await.unwrap();

        let first = store
            .create_pending_request(identity.agent_id.clone(), target("surf_1"), vec![ActionClass::Drive], None, NOW + 1)
            .await
            .unwrap();
        let second = store
            .create_pending_request(identity.agent_id, target("surf_1"), vec![ActionClass::Drive], None, NOW + 2)
            .await
            .unwrap();

        assert_eq!(first.request_id, second.request_id);
        assert_eq!(store.debug_permission_request_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn pending_request_idempotency_canonicalizes_action_ordering() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let identity = store.create_identity("agent", None, NOW).await.unwrap();

        let first = store
            .create_pending_request(
                identity.agent_id.clone(),
                target("surf_1"),
                vec![ActionClass::Drive, ActionClass::Observe],
                None,
                NOW + 1,
            )
            .await
            .unwrap();
        let second = store
            .create_pending_request(
                identity.agent_id,
                target("surf_1"),
                vec![ActionClass::Observe, ActionClass::Drive],
                None,
                NOW + 2,
            )
            .await
            .unwrap();

        assert_eq!(first.request_id, second.request_id);
        assert_eq!(second.actions, vec![ActionClass::Observe, ActionClass::Drive]);
        assert_eq!(store.debug_permission_request_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn approving_request_creates_matching_grant_and_marks_request_resolved() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let request = pending_request(&store).await;

        let grant = store
            .approve_request(
                &request.request_id,
                DurationSpec::UntilSurfaceGone,
                vec![Constraint::RequiresFrontmost],
                NOW + 2,
            )
            .await
            .unwrap();
        let resolved = store.get_permission_request(&request.request_id).await.unwrap().unwrap();
        let snapshot = store.load_policy_snapshot(&request.agent_id).await.unwrap();

        assert_eq!(grant.origin_request_id, Some(request.request_id));
        assert_eq!(resolved.status, PermissionRequestStatus::Approved);
        assert_eq!(snapshot.grants.len(), 1);
        assert_eq!(snapshot.grants[0].target, target("surf_1"));
        assert_eq!(snapshot.grants[0].actions, vec![ActionClass::Drive]);
        assert_eq!(snapshot.grants[0].constraints, vec![Constraint::RequiresFrontmost]);
    }

    #[tokio::test]
    async fn list_active_grants_excludes_revoked_consumed_and_expired_grants() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let live_request = pending_request(&store).await;
        let live_grant = store
            .approve_request(&live_request.request_id, DurationSpec::Persistent, Vec::new(), NOW + 2)
            .await
            .unwrap();

        let revoked_request = store
            .create_pending_request(
                live_request.agent_id.clone(),
                target("surf_2"),
                vec![ActionClass::Drive],
                None,
                NOW + 3,
            )
            .await
            .unwrap();
        let revoked_grant = store
            .approve_request(&revoked_request.request_id, DurationSpec::Persistent, Vec::new(), NOW + 4)
            .await
            .unwrap();
        store.revoke_grant(&revoked_grant.grant_id, NOW + 5).await.unwrap();

        let expired_request = store
            .create_pending_request(
                live_request.agent_id.clone(),
                target("surf_3"),
                vec![ActionClass::Drive],
                None,
                NOW + 6,
            )
            .await
            .unwrap();
        store
            .approve_request(
                &expired_request.request_id,
                DurationSpec::TimeBounded {
                    expires_at_unix_ms: NOW + 7,
                },
                Vec::new(),
                NOW + 6,
            )
            .await
            .unwrap();

        let once_request = store
            .create_pending_request(live_request.agent_id, target("surf_4"), vec![ActionClass::Drive], None, NOW + 8)
            .await
            .unwrap();
        let once_grant = store
            .approve_request(&once_request.request_id, DurationSpec::Once, Vec::new(), NOW + 9)
            .await
            .unwrap();
        store.try_consume_once_grant(&once_grant.grant_id, NOW + 10).await.unwrap();

        let active = store.list_active_grants(NOW + 11).await.unwrap();

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].grant_id, live_grant.grant_id);
    }

    #[tokio::test]
    async fn denying_request_persists_reason_for_denial_and_audit_rows() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let request = store
            .create_pending_request(
                store.create_identity("agent", None, NOW).await.unwrap().agent_id,
                target("surf_1"),
                vec![ActionClass::Drive],
                Some("needs typing".into()),
                NOW + 1,
            )
            .await
            .unwrap();

        store
            .deny_request(&request.request_id, true, Some("operator denied".into()), NOW + 2)
            .await
            .unwrap();

        assert_eq!(store.debug_denial_reasons().await.unwrap(), vec![Some("operator denied".into())]);
        assert_eq!(
            store.debug_audit_reasons().await.unwrap(),
            vec![Some("needs typing".into()), Some("operator denied".into())]
        );
    }

    #[tokio::test]
    async fn denial_takes_precedence_in_store_snapshot() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let request = pending_request(&store).await;
        store
            .approve_request(&request.request_id, DurationSpec::Persistent, Vec::new(), NOW + 2)
            .await
            .unwrap();
        let second = store
            .create_pending_request(request.agent_id.clone(), target("surf_1"), vec![ActionClass::Drive], None, NOW + 3)
            .await
            .unwrap();
        store
            .deny_request(&second.request_id, true, Some("no".into()), NOW + 4)
            .await
            .unwrap();

        let snapshot = store.load_policy_snapshot(&request.agent_id).await.unwrap();
        let decision = snapshot.authorize(
            &AgentContext {
                agent_id: request.agent_id,
            },
            &target_context("surf_1"),
            &[ActionClass::Drive],
            NOW + 5,
        );

        assert!(matches!(decision, AuthorizationDecision::Denied { .. }));
    }

    #[tokio::test]
    async fn audit_rows_are_written_for_requested_approved_denied_and_executed_decisions() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let approved_request = pending_request(&store).await;
        let grant = store
            .approve_request(&approved_request.request_id, DurationSpec::Persistent, Vec::new(), NOW + 2)
            .await
            .unwrap();
        store
            .insert_execution_audit(ExecutionAuditRecord {
                agent_id: approved_request.agent_id.clone(),
                request_id: Some(approved_request.request_id),
                route: "/surfaces/{id}/text".into(),
                target: target("surf_1"),
                actions: vec![ActionClass::Drive],
                grant_id: Some(grant.grant_id),
                denial_id: None,
                executed_at_unix_ms: NOW + 3,
            })
            .await
            .unwrap();
        let denied_request = store
            .create_pending_request(approved_request.agent_id, target("surf_2"), vec![ActionClass::Drive], None, NOW + 4)
            .await
            .unwrap();
        store.deny_request(&denied_request.request_id, true, None, NOW + 5).await.unwrap();

        let decisions = store.debug_audit_decisions().await.unwrap();

        assert_eq!(decisions, vec!["requested", "approved", "executed", "requested", "denied"]);
    }

    #[tokio::test]
    async fn once_grant_is_atomically_consumed_before_execution() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let request = pending_request(&store).await;
        let grant = store
            .approve_request(&request.request_id, DurationSpec::Once, Vec::new(), NOW + 2)
            .await
            .unwrap();

        let first = store.try_consume_once_grant(&grant.grant_id, NOW + 3).await.unwrap();
        let second = store.try_consume_once_grant(&grant.grant_id, NOW + 4).await.unwrap();
        let snapshot = store.load_policy_snapshot(&request.agent_id).await.unwrap();

        assert!(first);
        assert!(!second);
        assert_eq!(snapshot.consumed_once_grants, vec![GrantId::from(grant.grant_id.as_str())]);
    }

    #[tokio::test]
    async fn token_lookup_parses_visible_agent_id_prefix_and_does_not_full_scan_token_hashes() {
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let first = store.create_identity("first", None, NOW).await.unwrap();
        let second = store.create_identity("second", None, NOW).await.unwrap();
        let forged_token = format!("pta_{}.secret-known-only-on-second-row", first.agent_id);
        store
            .debug_insert_token_hash_for_agent(&second.agent_id, &forged_token, NOW + 1)
            .await
            .unwrap();

        let authenticated = store.authenticate_agent_token(&forged_token).await.unwrap();

        assert_eq!(authenticated, None);
    }
}
