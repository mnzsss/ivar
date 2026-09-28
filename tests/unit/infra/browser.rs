use crate::infra::browser;

#[test]
fn opener_constructs_platform_command() {
    let cmd = browser::opener("https://example.com");
    #[cfg(target_os = "macos")]
    {
        assert_eq!(cmd.program(), "open");
        assert_eq!(cmd.arguments(), &["https://example.com"]);
    }
    #[cfg(target_os = "windows")]
    {
        assert_eq!(cmd.program(), "cmd");
        assert_eq!(cmd.arguments(), &["/c", "start", "", "https://example.com"]);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        assert_eq!(cmd.program(), "xdg-open");
        assert_eq!(cmd.arguments(), &["https://example.com"]);
    }
}
