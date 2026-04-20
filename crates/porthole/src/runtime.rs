use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PORTHOLE_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole.sock");
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole").join("porthole.sock");
    }
    if let Ok(tmp) = std::env::var("TMPDIR") {
        let uid = unsafe { libc_getuid() };
        return PathBuf::from(tmp).join(format!("porthole-{uid}")).join("porthole.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from("/tmp").join(format!("porthole-{uid}")).join("porthole.sock")
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}
