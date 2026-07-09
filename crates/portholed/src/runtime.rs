pub fn control_endpoint() -> porthole_transport::Endpoint {
    porthole_transport::control_endpoint()
}

#[cfg(unix)]
pub fn socket_path() -> std::path::PathBuf {
    match control_endpoint() {
        porthole_transport::Endpoint::Unix(path) => path,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn porthole_runtime_dir_wins() {
        // SAFETY: tests are serial-friendly via distinct env var names; this
        // test only touches PORTHOLE_RUNTIME_DIR.
        // Note: set_var is marked unsafe in edition 2024; we accept this in tests.
        unsafe {
            std::env::set_var("PORTHOLE_RUNTIME_DIR", "/tmp/test-porthole");
        }
        let p = socket_path();
        assert_eq!(p, PathBuf::from("/tmp/test-porthole/porthole.sock"));
        unsafe {
            std::env::remove_var("PORTHOLE_RUNTIME_DIR");
        }
    }
}
