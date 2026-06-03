use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::{ErrorCode, PortholeError, surface::PlatformSurfaceRef};

/// Query passed to `Adapter::search`. Every field is optional; matching is
/// AND across fields, OR within a list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub title_pattern: Option<String>,
    #[serde(default)]
    pub pids: Vec<u32>,
    #[serde(default)]
    pub platform_refs: Vec<PlatformSurfaceRef>,
    #[serde(default)]
    pub frontmost: Option<bool>,
}

/// A window that matched a search. Opaque `ref` carries enough state to
/// re-identify the window in a later `track` call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub platform_ref: PlatformSurfaceRef,
}

const REF_PREFIX: &str = "ref_";
const REF_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RefPayload {
    pid: u32,
    platform_ref: PlatformSurfaceRef,
    v: u32,
}

/// Encode a platform surface identity into a self-describing opaque ref.
pub fn encode_ref(pid: u32, platform_ref: PlatformSurfaceRef) -> String {
    let payload = RefPayload {
        pid,
        platform_ref,
        v: REF_SCHEMA_VERSION,
    };
    let json = serde_json::to_vec(&payload).expect("RefPayload is JSON-serialisable");
    format!("{REF_PREFIX}{}", URL_SAFE_NO_PAD.encode(json))
}

/// Decode a ref back to its platform surface identity. Returns
/// `candidate_ref_unknown`
/// on any structural failure (wrong prefix, bad base64, bad JSON, unknown
/// schema version).
pub fn decode_ref(r: &str) -> Result<(u32, PlatformSurfaceRef), PortholeError> {
    let body = r
        .strip_prefix(REF_PREFIX)
        .ok_or_else(|| PortholeError::new(ErrorCode::CandidateRefUnknown, format!("ref missing '{REF_PREFIX}' prefix")))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(body)
        .map_err(|e| PortholeError::new(ErrorCode::CandidateRefUnknown, format!("ref base64 decode failed: {e}")))?;
    let payload: RefPayload = serde_json::from_slice(&bytes)
        .map_err(|e| PortholeError::new(ErrorCode::CandidateRefUnknown, format!("ref JSON decode failed: {e}")))?;
    if payload.v != REF_SCHEMA_VERSION {
        return Err(PortholeError::new(
            ErrorCode::CandidateRefUnknown,
            format!("ref schema version {} is not supported (expected {REF_SCHEMA_VERSION})", payload.v),
        ));
    }
    Ok((payload.pid, payload.platform_ref))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let platform_ref = PlatformSurfaceRef::macos(42);
        let r = encode_ref(9876, platform_ref.clone());
        let (pid, decoded_ref) = decode_ref(&r).unwrap();
        assert_eq!(pid, 9876);
        assert_eq!(decoded_ref, platform_ref);
    }

    #[test]
    fn encoded_ref_has_prefix() {
        assert!(encode_ref(1, PlatformSurfaceRef::macos(1)).starts_with("ref_"));
    }

    #[test]
    fn decode_missing_prefix_errors() {
        let err = decode_ref("abc").unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[test]
    fn decode_bad_base64_errors() {
        let err = decode_ref("ref_!!!not-base64!!!").unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[test]
    fn decode_bad_json_errors() {
        let payload = URL_SAFE_NO_PAD.encode(b"not-json");
        let err = decode_ref(&format!("ref_{payload}")).unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }

    #[test]
    fn decode_wrong_schema_version_errors() {
        let payload =
            serde_json::to_vec(&serde_json::json!({ "pid": 1, "platform_ref": { "platform": "macos", "cg_window_id": 1 }, "v": 99 }))
                .unwrap();
        let encoded = URL_SAFE_NO_PAD.encode(payload);
        let err = decode_ref(&format!("ref_{encoded}")).unwrap_err();
        assert_eq!(err.code, ErrorCode::CandidateRefUnknown);
    }
}
