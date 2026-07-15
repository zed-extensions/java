use std::path::PathBuf;

use zed_extension_api::{self as zed, LanguageServerId, Worktree, serde_json::Value};

use crate::util::should_use_local_or_download;

pub trait Downloadable {
    const INSTALL_PATH: &'static str;

    fn find_local(&self) -> Option<PathBuf>;

    fn loaded(&self) -> bool;

    fn fetch_latest_version(&self) -> zed::Result<String>;

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
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
            .fetch_latest_version()
            .and_then(|version| self.download(&version, language_server_id));

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
