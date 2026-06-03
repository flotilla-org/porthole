// Note: unsafe_code is allowed in this crate because runtime.rs uses a libc
// FFI shim (getuid) to derive per-user socket paths.  All other crates that
// have no unsafe requirements keep #![forbid(unsafe_code)].

pub mod agent_store;
pub mod capture_registry;
pub mod events;
#[cfg(target_os = "linux")]
pub mod kwin_bridge;
pub mod routes;
pub mod runtime;
pub mod server;
pub mod state;
