use std::path::PathBuf;

use zed_extension_api::{self as zed, LanguageServerId, Worktree, serde_json::Value};

use crate::util::should_use_local_or_download;

pub trait Downloadable {
    const INSTALL_PATH: &'static str;

    fn find_local(&self) -> Option<PathBuf>;

    fn loaded(&self) -> bool;

    fn fetch_latest_version(&self, worktree: &Worktree) -> zed::Result<String>;

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf>;

    fn get_or_download(
        &mut self,
        language_server_id: &LanguageServerId,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        if let Some(path) = self.user_configured_path(configuration, worktree) {
            return Ok(PathBuf::from(path));
        }

        if let Some(path) =
            should_use_local_or_download(configuration, self.find_local(), Self::INSTALL_PATH)?
        {
            return Ok(path);
        }

        let downloaded = self
            .fetch_latest_version(worktree)
            .and_then(|version| self.download(&version, language_server_id, worktree));

        match downloaded {
            Ok(path) => Ok(path),
            // The version check or download failed (e.g. GitHub API rate
            // limiting) — an existing local installation is better than none.
            Err(err) => match self.find_local() {
                Some(path) => {
                    println!(
                        "Failed to update {}, falling back to local installation: {err}",
                        Self::INSTALL_PATH
                    );
                    Ok(path)
                }
                None => Err(err),
            },
        }
    }

    fn user_configured_path(
        &self,
        _configuration: &Option<Value>,
        _worktree: &Worktree,
    ) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod fallback_tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    #[test]
    fn test_check_updates_always_allows_download() {
        let result = should_use_local_or_download(&None, None, "jdtls").unwrap();
        assert!(result.is_none(), "Always mode should allow download");
    }

    #[test]
    fn test_check_updates_always_with_local_still_downloads() {
        let local = PathBuf::from("/mock/jdtls/1.44.0");
        let result = should_use_local_or_download(&None, Some(local), "jdtls").unwrap();
        assert!(result.is_none(), "Always mode downloads even with local");
    }

    #[test]
    fn test_check_updates_never_with_local_uses_it() {
        let config = Some(json!({"check_updates": "never"}));
        let local = PathBuf::from("/mock/jdtls/1.44.0");
        let result = should_use_local_or_download(&config, Some(local.clone()), "jdtls").unwrap();
        assert_eq!(result, Some(local));
    }

    #[test]
    fn test_check_updates_never_without_local_is_error() {
        let config = Some(json!({"check_updates": "never"}));
        let result = should_use_local_or_download(&config, None, "jdtls");
        assert!(result.is_err());
    }

    #[test]
    fn test_check_updates_once_with_local_uses_it() {
        let config = Some(json!({"check_updates": "once"}));
        let local = PathBuf::from("/mock/jdtls/1.44.0");
        let result = should_use_local_or_download(&config, Some(local.clone()), "jdtls").unwrap();
        assert_eq!(result, Some(local));
    }

    #[test]
    fn test_default_is_always() {
        let result = should_use_local_or_download(&None, None, "test").unwrap();
        assert!(result.is_none(), "Default should be Always (None)");
    }
}
