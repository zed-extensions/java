use std::env;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
pub fn get_jdtls_cache_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg);
    }
    env::var("HOME")
        .map(|h| PathBuf::from(h).join("Library").join("Caches"))
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
pub fn get_jdtls_cache_dir() -> PathBuf {
    env::var("APPDATA")
        .map(|h| PathBuf::from(h).join("Java").join("JDTLS"))
        .unwrap_or_default()
}

#[cfg(unix)]
pub fn get_jdtls_cache_dir() -> PathBuf {
    if let Ok(xdg) = env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg);
    }
    env::var("HOME")
        .map(|h| PathBuf::from(h).join(".cache"))
        .unwrap_or_default()
}
