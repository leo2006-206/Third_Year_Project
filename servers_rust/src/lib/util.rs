use std::sync::OnceLock;

static LOCAL_URL: OnceLock<String> = OnceLock::new();

/// Accepts any string slice or owned string at startup
pub fn set_local_url(url: impl Into<String>) -> Result<(), &'static str> {
    LOCAL_URL
        .set(url.into())
        .map_err(|_| "LOCAL_URL is already set")
}

/// Returns a reference that lives as long as the program (`'static`)
pub fn local_url() -> Option<&'static str> {
    LOCAL_URL.get().map(|s| s.as_str())
}
