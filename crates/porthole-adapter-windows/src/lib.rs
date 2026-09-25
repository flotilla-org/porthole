//! Windows desktop operations. Continuous native capture runs in portholed,
//! which asks this adapter for the verified window (`native_capture_window`).
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::WindowsAdapter;

mod keys;
