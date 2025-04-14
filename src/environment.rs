#[cfg(not(feature = "test-integration"))]
pub fn is_enabled(name: &str) -> bool {
    let value = std::env::var(name).unwrap_or_else(|_| "0".to_string());
    value == "1" || value == "true"
}

// NOTE This is necessary to avoid conflicts with environment variables
// In the tests/common/macros.rs the variables are set to default
#[cfg(feature = "test-integration")]
pub fn is_enabled(name: &str) -> bool {
    let value = std::env::var(format!("_TEST_{}", name)).unwrap_or_else(|_| "0".to_string());
    value == "1" || value == "true"
}
