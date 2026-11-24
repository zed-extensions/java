use std::{
    fs,
    path::{Path, PathBuf},
};

use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Worktree,
    serde_json::{Value, json},
    set_language_server_installation_status,
};

use crate::downloadable::Downloadable;

const VERSION: &str = "0.47.0";

pub fn prepare(
    language_server_id: &LanguageServerId,
    worktree: &Worktree,
    host: Option<&Path>,
) -> zed::Result<String> {
    crate::lsp::request::<Value>(
        &worktree.root_path(),
        "workspace/executeCommand",
        json!({"command": "java.project.getAll", "arguments": []}),
    )
    .map_err(|error| {
        format!("JDTLS is not responding for this workspace. Check its launcher and language server log, then restart JDTLS: {error}")
    })?;
    let mut test_runner = TestRunner;
    let directory = test_runner.get_or_download(language_server_id, &None, worktree)?;
    let (mut bundles, runner) = load_installation(&directory)?;
    if let Some(host) = host {
        let plugins = host.join("plugins");
        bundles.retain(|bundle| {
            !Path::new(bundle)
                .file_name()
                .is_some_and(|name| plugins.join(name).is_file())
        });
    }
    let loaded = crate::lsp::request::<bool>(
        &worktree.root_path(),
        "workspace/executeCommand",
        json!({"command": "java.reloadBundles", "arguments": [bundles]}),
    )
    .map_err(|error| format!("Failed to load Java Test Runner in JDTLS: {error}"))?;
    if !loaded {
        return Err("JDTLS could not load the Java Test Runner; check the JDTLS log".into());
    }
    Ok(runner)
}

struct TestRunner;

impl Downloadable for TestRunner {
    const INSTALL_PATH: &'static str = "java-test";

    fn get_or_download(
        &mut self,
        language_server_id: &LanguageServerId,
        _configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        match self.find_local() {
            Some(path) => Ok(path),
            None => self.download(VERSION, language_server_id, worktree),
        }
    }

    fn find_local(&self) -> Option<PathBuf> {
        let directory = std::env::current_dir()
            .ok()?
            .join(Self::INSTALL_PATH)
            .join(VERSION);
        load_installation(&directory).ok().map(|_| directory)
    }

    fn loaded(&self) -> bool {
        self.find_local().is_some()
    }

    fn fetch_latest_version(&self, _worktree: &Worktree) -> zed::Result<String> {
        Ok(VERSION.to_owned())
    }

    fn version_for_download(
        &self,
        _language_server_id: &LanguageServerId,
        _configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> zed::Result<(String, bool)> {
        self.fetch_latest_version(worktree)
            .map(|version| (version, false))
    }

    fn download(
        &mut self,
        version: &str,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        let directory = std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(Self::INSTALL_PATH)
            .join(version);
        let staging = directory.with_extension("installing");
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
        }
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        let url = format!(
            "https://open-vsx.org/api/vscjava/vscode-java-test/{version}/file/vscjava.vscode-java-test-{version}.vsix"
        );
        let downloaded =
            zed::download_file(&url, &staging.to_string_lossy(), DownloadedFileType::Zip);
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );
        downloaded
            .map_err(|error| format!("Failed to install Java Test Runner {version}: {error}"))?;
        load_installation(&staging)?;
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        }
        fs::rename(staging, &directory).map_err(|error| error.to_string())?;
        Ok(directory)
    }
}

fn load_installation(directory: &std::path::Path) -> zed::Result<(Vec<String>, String)> {
    let extension = directory.join("extension");
    let manifest: Value = serde_json::from_slice(
        &fs::read(extension.join("package.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let paths = manifest
        .pointer("/contributes/javaExtensions")
        .and_then(Value::as_array)
        .ok_or("Java Test Runner manifest has no Java extensions")?;
    if paths.is_empty() {
        return Err("Java Test Runner contains no language server bundles".into());
    }
    let bundles = paths
        .iter()
        .map(|path| {
            let path = path.as_str().ok_or("Invalid Java test bundle path")?;
            let relative = PathBuf::from(path);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err("Invalid Java test bundle path".into());
            }
            let path = extension.join(relative);
            if !path.is_file() {
                return Err(format!("Java test bundle is missing: {}", path.display()));
            }
            Ok(path.to_string_lossy().into_owned())
        })
        .collect::<zed::Result<Vec<_>>>()?;
    let runner = extension.join("server/com.microsoft.java.test.runner-jar-with-dependencies.jar");
    if !runner.is_file() {
        return Err("Java TestNG runner is missing".into());
    }
    Ok((bundles, runner.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_rejects_bundle_paths_outside_the_extension() {
        let directory =
            std::env::temp_dir().join(format!("java-test-manifest-{}", std::process::id()));
        fs::create_dir_all(directory.join("extension")).unwrap();
        fs::write(
            directory.join("extension/package.json"),
            json!({
                "contributes": {"javaExtensions": ["../../outside.jar"]}
            })
            .to_string(),
        )
        .unwrap();

        let error = load_installation(&directory).unwrap_err();

        assert!(error.contains("Invalid Java test bundle path"));
        fs::remove_dir_all(directory).unwrap();
    }
}
