pub const PORTHOLE_LAUNCH_TAG_ENV: &str = "PORTHOLE_LAUNCH_TAG";

pub fn new_launch_tag() -> String {
    format!("plt_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_launch_tag_has_prefix() {
        assert!(new_launch_tag().starts_with("plt_"));
    }
}
