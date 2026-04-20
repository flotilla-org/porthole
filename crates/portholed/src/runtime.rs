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

#[cfg(test)]
mod tests {
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
