//! One-shot Windows desktop operations. No continuous capture transport.
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::WindowsAdapter;

mod keys;
