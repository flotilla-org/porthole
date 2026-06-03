use std::sync::Arc;

use crate::{
    ErrorCode, PortholeError,
    adapter::Adapter,
    handle::HandleStore,
    search::{Candidate, SearchQuery, decode_ref},
    surface::SurfaceInfo,
};

pub struct AttachPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

#[derive(Debug)]
pub struct TrackedOutcome {
    pub surface: SurfaceInfo,
    pub reused_existing_handle: bool,
}

impl AttachPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
        if let Some(p) = &query.title_pattern {
            if let Err(e) = regex::Regex::new(p) {
                return Err(PortholeError::new(
                    ErrorCode::InvalidArgument,
                    format!("invalid title_pattern regex: {e}"),
                ));
            }
        }
        self.adapter.search(query).await
    }

    pub async fn track(&self, r: &str) -> Result<TrackedOutcome, PortholeError> {
        let (pid, platform_ref) = decode_ref(r)?;
        let info = self.adapter.surface_alive(pid, &platform_ref).await?.ok_or_else(|| {
            PortholeError::new(
                ErrorCode::SurfaceDead,
                format!("surface with platform ref {platform_ref:?} (pid {pid}) is no longer alive"),
            )
        })?;
        let (surface, reused) = self.handles.track_or_get(info).await;
        Ok(TrackedOutcome {
            surface,
            reused_existing_handle: reused,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        in_memory::InMemoryAdapter,
        search::encode_ref,
        surface::{PlatformSurfaceRef, SurfaceId},
    };

    fn surface_with_ref(pid: u32, platform_ref: PlatformSurfaceRef) -> SurfaceInfo {
        let mut info = SurfaceInfo::window(SurfaceId::new(), pid);
        info.platform_ref = Some(platform_ref);
        info
    }

    #[tokio::test]
    async fn track_decodes_ref_and_dispatches_to_adapter() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let platform_ref = PlatformSurfaceRef::macos(42);
        adapter
            .set_next_surface_alive_result(Ok(Some(surface_with_ref(9876, platform_ref.clone()))))
            .await;
        let pipeline = AttachPipeline::new(adapter.clone(), HandleStore::new());
        let r = encode_ref(9876, platform_ref.clone());
        let out = pipeline.track(&r).await.unwrap();
        assert!(!out.reused_existing_handle);
        assert_eq!(out.surface.platform_ref, Some(platform_ref.clone()));
        let calls = adapter.surface_alive_calls().await;
        assert_eq!(calls, vec![(9876, platform_ref)]);
    }

    #[tokio::test]
    async fn track_returns_surface_dead_when_window_gone() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_next_surface_alive_result(Ok(None)).await;
        let pipeline = AttachPipeline::new(adapter, HandleStore::new());
        let r = encode_ref(1, PlatformSurfaceRef::macos(1));
        let err = pipeline.track(&r).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn track_errors_on_malformed_ref() {
        let pipeline = AttachPipeline::new(Arc::new(InMemoryAdapter::new()), HandleStore::new());
        let err = pipeline.track("not-a-ref").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[tokio::test]
    async fn track_reuses_existing_alive_handle() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter
            .set_next_surface_alive_result(Ok(Some(surface_with_ref(1, PlatformSurfaceRef::macos(7)))))
            .await;
        let handles = HandleStore::new();
        let pipeline = AttachPipeline::new(adapter.clone(), handles.clone());
        let r = encode_ref(1, PlatformSurfaceRef::macos(7));
        let first = pipeline.track(&r).await.unwrap();

        // Second call — adapter returns a different SurfaceInfo (fresh id),
        // but track_or_get should return the first one.
        adapter
            .set_next_surface_alive_result(Ok(Some(surface_with_ref(1, PlatformSurfaceRef::macos(7)))))
            .await;
        let second = pipeline.track(&r).await.unwrap();
        assert!(second.reused_existing_handle);
        assert_eq!(second.surface.id, first.surface.id);
    }

    #[tokio::test]
    async fn search_rejects_invalid_regex() {
        let pipeline = AttachPipeline::new(Arc::new(InMemoryAdapter::new()), HandleStore::new());
        let err = pipeline
            .search(&SearchQuery {
                title_pattern: Some("[invalid".into()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }
}
