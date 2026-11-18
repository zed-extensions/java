use std::{
    fs,
    path::{Path, PathBuf},
};

use zed_extension_api::{
    self as zed, DownloadedFileType,
    serde_json::{Value, json},
};

const VERSION: &str = "0.47.0";
const DOWNLOAD_URL: &str = "https://open-vsx.org/api/vscjava/vscode-java-test/0.47.0/file/vscjava.vscode-java-test-0.47.0.vsix";

pub fn prepare(workspace: &str, host: Option<&Path>) -> zed::Result<String> {
    let (mut bundles, runner) = install()?;
    if let Some(host) = host {
        let plugins = host.join("plugins");
        bundles.retain(|bundle| {
            !Path::new(bundle)
                .file_name()
                .is_some_and(|name| plugins.join(name).is_file())
        });
    }
    let loaded = crate::lsp::request::<bool>(
        workspace,
        "workspace/executeCommand",
        json!({"command": "java.reloadBundles", "arguments": [bundles]}),
    )
    .map_err(|error| format!("Failed to load Java Test Runner in JDTLS: {error}"))?;
    if !loaded {
        return Err("JDTLS could not load the Java Test Runner; check the JDTLS log".into());
    }
    Ok(runner)
}

fn install() -> zed::Result<(Vec<String>, String)> {
    let directory = std::env::current_dir()
        .map_err(|error| error.to_string())?
        .join("java-test")
        .join(VERSION);
    if load_installation(&directory).is_err() {
        let staging = directory.with_extension("installing");
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
        }
        zed::download_file(
            DOWNLOAD_URL,
            &staging.to_string_lossy(),
            DownloadedFileType::Zip,
        )
        .map_err(|error| format!("Failed to install Java Test Runner {VERSION}: {error}"))?;
        load_installation(&staging)?;
        if directory.exists() {
            fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
        }
        fs::rename(staging, &directory).map_err(|error| error.to_string())?;
    }
    load_installation(&directory)
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
