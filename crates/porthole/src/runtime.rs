pub fn control_endpoint() -> porthole_transport::Endpoint {
    porthole_transport::control_endpoint()
}

#[cfg(unix)]
pub fn socket_path() -> std::path::PathBuf {
    match control_endpoint() {
        porthole_transport::Endpoint::Unix(path) => path,
    }
}

#[cfg(windows)]
pub fn socket_path() -> std::path::PathBuf {
    std::path::PathBuf::from(control_endpoint().display_name())
}
