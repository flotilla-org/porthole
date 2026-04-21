use std::sync::Arc;

use crate::adapter::Adapter;
use crate::handle::HandleStore;
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::key_names;
use crate::surface::SurfaceId;
use crate::{ErrorCode, PortholeError};

pub struct InputPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl InputPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn key(&self, surface: &SurfaceId, events: &[KeyEvent]) -> Result<(), PortholeError> {
        for ev in events {
            if !key_names::is_supported(&ev.key) {
                return Err(PortholeError::new(
                    ErrorCode::UnknownKey,
                    format!("key '{}' is not in the supported set", ev.key),
                ));
            }
        }
        let info = self.handles.require_alive(surface).await?;
        self.adapter.key(&info, events).await
    }

    pub async fn text(&self, surface: &SurfaceId, text: &str) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.text(&info, text).await
    }

    pub async fn click(&self, surface: &SurfaceId, spec: &ClickSpec) -> Result<(), PortholeError> {
        if spec.count == 0 || spec.count > 3 {
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                format!("click count must be 1, 2, or 3 (got {})", spec.count),
            ));
        }
        let info = self.handles.require_alive(surface).await?;
        self.adapter.click(&info, spec).await
    }

    pub async fn scroll(&self, surface: &SurfaceId, spec: &ScrollSpec) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.scroll(&info, spec).await
    }

    pub async fn close(&self, surface: &SurfaceId) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.close(&info).await?;
        self.handles.mark_dead(surface).await?;
        Ok(())
    }

    pub async fn focus(&self, surface: &SurfaceId) -> Result<(), PortholeError> {
        let info = self.handles.require_alive(surface).await?;
        self.adapter.focus(&info).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryAdapter;
    use crate::surface::SurfaceInfo;

    async fn setup() -> (Arc<InMemoryAdapter>, HandleStore, SurfaceId) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        (adapter, handles, id)
    }

    #[tokio::test]
    async fn key_rejects_unsupported_name() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline.key(&id, &[KeyEvent { key: "NotAKey".into(), modifiers: vec![] }]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownKey);
    }

    #[tokio::test]
    async fn key_delegates_to_adapter() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        pipeline.key(&id, &[KeyEvent { key: "Enter".into(), modifiers: vec![] }]).await.unwrap();
        assert_eq!(adapter.key_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn click_rejects_count_zero() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles);
        let err = pipeline
            .click(&id, &ClickSpec { x: 0.0, y: 0.0, button: crate::input::ClickButton::Left, count: 0, modifiers: vec![] })
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
    }

    #[tokio::test]
    async fn close_marks_handle_dead() {
        let (adapter, handles, id) = setup().await;
        let pipeline = InputPipeline::new(adapter.clone(), handles.clone());
        pipeline.close(&id).await.unwrap();
        let err = handles.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }
}
