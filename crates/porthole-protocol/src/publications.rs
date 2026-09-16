//! Publications and exports.
//!
//! A *publication* is a running frame stream with a local endpoint: today every
//! capture session is one, and a *republished* publication is the local end of
//! a cross-host bridge. An *export* is the graph-manager operation that creates
//! that bridge's producer-side half for a publication; the consumer side
//! creates the matching republication from the forwarded sockets. Vocabulary
//! follows `jackstay-graph`; identities keep the captured source apart from the
//! running stream that shows it.

pub use jackstay_graph::{Chroma, ChromaPolicy, Codec, CodecDecision, Identities};
use serde::{Deserialize, Serialize};

use crate::capture_sessions::NativeCaptureInfo;

pub const PUBLICATION_KIND_CAPTURE: &str = "capture";
pub const PUBLICATION_KIND_REPUBLISHED: &str = "republished";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicationResponse {
    /// The capture session id for captures; a fresh id for republications.
    pub publication_id: String,
    pub kind: String,
    pub identities: Identities,
    pub status: String,
    pub status_message: Option<String>,
    pub width: u32,
    pub height: u32,
    /// How a native consumer attaches, when the publication is native.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeCaptureInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListPublicationsResponse {
    pub publications: Vec<PublicationResponse>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreateExportRequest {
    #[serde(default)]
    pub chroma: ChromaPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_bps: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportResponse {
    pub export_id: String,
    pub publication_id: String,
    pub identities: Identities,
    /// `starting`, `listening`, `running`, `ended`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<CodecDecision>,
    /// Sockets the egress half listens on; the consumer side forwards them.
    pub media_socket: String,
    pub control_socket: String,
    /// The token both halves check; hand it to the consumer side only.
    pub link_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepublishRequest {
    /// Local ends of the forwarded export sockets.
    pub media_socket: String,
    pub control_socket: String,
    pub link_token: String,
    /// Identities of the exported publication, carried over unchanged.
    pub identities: Identities,
    #[serde(default)]
    pub chroma: ChromaPolicy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepublishResponse {
    pub publication: PublicationResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<CodecDecision>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_export_request_defaults_and_rejects_unknown_fields() {
        let r: CreateExportRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(r.chroma, ChromaPolicy::Prefer444);
        assert!(serde_json::from_str::<CreateExportRequest>(r#"{"carrier":"x"}"#).is_err());
    }

    #[test]
    fn republish_request_round_trips() {
        let r = RepublishRequest {
            media_socket: "/m".into(),
            control_socket: "/c".into(),
            link_token: "L".into(),
            identities: Identities {
                source: "surf_1".into(),
                publication: "sess_1".into(),
            },
            chroma: ChromaPolicy::Any,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<RepublishRequest>(&json).unwrap(), r);
    }
}
