use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    ErrorCode, PortholeError,
    surface::{PlatformSurfaceRef, SurfaceId, SurfaceInfo, SurfaceState},
};

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

    /// Find the first alive surface whose platform ref matches `platform_ref`.
    pub async fn find_by_platform_ref(&self, platform_ref: &PlatformSurfaceRef) -> Option<SurfaceId> {
        let guard = self.inner.read().await;
        guard
            .values()
            .find(|info| info.platform_ref.as_ref() == Some(platform_ref) && info.state == SurfaceState::Alive)
            .map(|info| info.id.clone())
    }

    pub async fn alive_count(&self) -> usize {
        let guard = self.inner.read().await;
        guard.values().filter(|info| info.state == SurfaceState::Alive).count()
    }

    /// Atomic get-or-insert keyed by platform ref. Holds the write lock
    /// across both the lookup and the insert so concurrent callers cannot
    /// both mint a new handle for the same window.
    ///
    /// Returns `(SurfaceInfo, reused)`:
    /// - If an alive tracked surface with this platform ref exists,
    ///   returns that surface with `reused = true`.
    /// - Otherwise inserts `candidate` and returns it with `reused = false`.
    ///
    /// Dead handles for the same platform ref are skipped — a fresh
    /// insert happens anyway, so re-tracking a window whose previous handle
    /// died returns a new surface id.
    pub async fn track_or_get(&self, candidate: SurfaceInfo) -> (SurfaceInfo, bool) {
        let mut guard = self.inner.write().await;
        if let Some(platform_ref) = candidate.platform_ref.as_ref() {
            for info in guard.values() {
                if info.platform_ref.as_ref() == Some(platform_ref) && info.state == SurfaceState::Alive {
                    return (info.clone(), true);
                }
            }
        }
        let key = candidate.id.clone();
        guard.insert(key, candidate.clone());
        (candidate, false)
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

    #[tokio::test]
    async fn platform_ref_returns_alive_surface() {
        let store = HandleStore::new();
        let mut info = SurfaceInfo::window(SurfaceId::new(), 42);
        info.platform_ref = Some(PlatformSurfaceRef::macos(9999));
        let id = info.id.clone();
        store.insert(info).await;
        let found = store.find_by_platform_ref(&PlatformSurfaceRef::macos(9999)).await;
        assert_eq!(found, Some(id));
    }

    #[tokio::test]
    async fn platform_ref_ignores_dead_surface() {
        let store = HandleStore::new();
        let mut info = SurfaceInfo::window(SurfaceId::new(), 42);
        info.platform_ref = Some(PlatformSurfaceRef::macos(8888));
        let id = info.id.clone();
        store.insert(info).await;
        store.mark_dead(&id).await.unwrap();
        let found = store.find_by_platform_ref(&PlatformSurfaceRef::macos(8888)).await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn platform_ref_returns_none_for_missing() {
        let store = HandleStore::new();
        let found = store.find_by_platform_ref(&PlatformSurfaceRef::macos(1234)).await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn track_or_get_is_atomic_under_concurrency() {
        use std::sync::Arc;

        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let cg_id: u32 = 4242;

        // N tasks all racing to track_or_get the same platform_ref with fresh
        // SurfaceInfo each time. Exactly one should see reused=false; the rest
        // reused=true. All should return the same surface_id.
        let n = 20;
        let mut tasks = Vec::with_capacity(n);
        let store = Arc::new(store);
        for _ in 0..n {
            let s = Arc::clone(&store);
            tasks.push(tokio::spawn(async move {
                let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
                info.platform_ref = Some(PlatformSurfaceRef::macos(cg_id));
                s.track_or_get(info).await
            }));
        }

        let mut observed_ids = std::collections::HashSet::new();
        let mut newly_minted = 0usize;
        for t in tasks {
            let (info, reused) = t.await.unwrap();
            observed_ids.insert(info.id);
            if !reused {
                newly_minted += 1;
            }
        }

        assert_eq!(newly_minted, 1, "exactly one task should have minted the handle");
        assert_eq!(observed_ids.len(), 1, "all tasks should see the same surface_id");
    }

    #[tokio::test]
    async fn track_or_get_inserts_when_absent() {
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
        info.platform_ref = Some(PlatformSurfaceRef::macos(99));
        let (returned, reused) = store.track_or_get(info.clone()).await;
        assert!(!reused);
        assert_eq!(returned.id, info.id);
    }

    #[tokio::test]
    async fn track_or_get_reuses_alive_handle() {
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let mut first = SurfaceInfo::window(SurfaceId::new(), 1);
        first.platform_ref = Some(PlatformSurfaceRef::macos(7));
        store.track_or_get(first.clone()).await;

        let mut second = SurfaceInfo::window(SurfaceId::new(), 1);
        second.platform_ref = Some(PlatformSurfaceRef::macos(7));
        let (returned, reused) = store.track_or_get(second).await;
        assert!(reused);
        assert_eq!(returned.id, first.id);
    }

    #[tokio::test]
    async fn track_or_get_skips_dead_handle() {
        use crate::surface::SurfaceInfo;

        let store = HandleStore::new();
        let mut dead = SurfaceInfo::window(SurfaceId::new(), 1);
        dead.platform_ref = Some(PlatformSurfaceRef::macos(5));
        let old_id = dead.id.clone();
        store.track_or_get(dead).await;
        store.mark_dead(&old_id).await.unwrap();

        let mut fresh = SurfaceInfo::window(SurfaceId::new(), 1);
        fresh.platform_ref = Some(PlatformSurfaceRef::macos(5));
        let (returned, reused) = store.track_or_get(fresh.clone()).await;
        assert!(!reused, "dead handle should not be reused");
        assert_eq!(returned.id, fresh.id);
    }

    #[tokio::test]
    async fn alive_count_excludes_dead_handles() {
        let store = HandleStore::new();
        let alive = SurfaceInfo::window(SurfaceId::new(), 1);
        let dead = SurfaceInfo::window(SurfaceId::new(), 2);
        let dead_id = dead.id.clone();
        store.insert(alive).await;
        store.insert(dead).await;
        store.mark_dead(&dead_id).await.unwrap();

        assert_eq!(store.alive_count().await, 1);
    }
}
