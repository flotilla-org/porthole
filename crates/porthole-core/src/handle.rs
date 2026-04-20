use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::surface::{SurfaceId, SurfaceInfo, SurfaceState};
use crate::{ErrorCode, PortholeError};

#[derive(Default, Clone)]
pub struct HandleStore {
    inner: Arc<RwLock<HashMap<SurfaceId, SurfaceInfo>>>,
}

impl HandleStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, info: SurfaceInfo) {
        let mut guard = self.inner.write().await;
        guard.insert(info.id.clone(), info);
    }

    pub async fn get(&self, id: &SurfaceId) -> Result<SurfaceInfo, PortholeError> {
        let guard = self.inner.read().await;
        guard.get(id).cloned().ok_or_else(|| PortholeError::surface_not_found(id.as_str()))
    }

    pub async fn mark_dead(&self, id: &SurfaceId) -> Result<(), PortholeError> {
        let mut guard = self.inner.write().await;
        match guard.get_mut(id) {
            Some(info) => {
                info.state = SurfaceState::Dead;
                Ok(())
            }
            None => Err(PortholeError::surface_not_found(id.as_str())),
        }
    }

    pub async fn require_alive(&self, id: &SurfaceId) -> Result<SurfaceInfo, PortholeError> {
        let info = self.get(id).await?;
        if info.state == SurfaceState::Dead {
            return Err(PortholeError::new(ErrorCode::SurfaceDead, format!("surface {id} is dead")));
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_then_get_roundtrips() {
        let store = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 42);
        let id = info.id.clone();
        store.insert(info).await;
        let fetched = store.get(&id).await.unwrap();
        assert_eq!(fetched.pid, Some(42));
    }

    #[tokio::test]
    async fn get_missing_returns_surface_not_found() {
        let store = HandleStore::new();
        let err = store.get(&SurfaceId::new()).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceNotFound);
    }

    #[tokio::test]
    async fn require_alive_fails_on_dead_surface() {
        let store = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        store.insert(info).await;
        store.mark_dead(&id).await.unwrap();
        let err = store.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }
}
