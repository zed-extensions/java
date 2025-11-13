use std::{
    collections::HashMap,
    fs::{self, metadata, read_dir},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus,
    TcpArgumentsTemplate, Worktree, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    serde_json::{self, Value, json},
    set_language_server_installation_status,
};

use crate::{
    lsp::LspWrapper,
    util::{create_path_if_not_exists, get_curr_dir, path_to_string, should_use_local_or_download},
};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct JavaDebugLaunchConfig {
    request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    main_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vm_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    class_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    module_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_on_entry: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    no_debug: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    console: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shorten_command_line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    launcher_script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    java_exec: Option<String>,
}

const TEST_SCOPE: &str = "$Test";
const AUTO_SCOPE: &str = "$Auto";
const RUNTIME_SCOPE: &str = "$Runtime";

const SCOPES: [&str; 3] = [TEST_SCOPE, AUTO_SCOPE, RUNTIME_SCOPE];

const DEBUGGER_INSTALL_PATH: &str = "debugger";

const JAVA_DEBUG_PLUGIN_FORK_URL: &str = "https://github.com/zed-industries/java-debug/releases/download/0.53.2/com.microsoft.java.debug.plugin-0.53.2.jar";

const MAVEN_METADATA_URL: &str = "https://repo1.maven.org/maven2/com/microsoft/java/com.microsoft.java.debug.plugin/maven-metadata.xml";

pub fn find_latest_local_debugger() -> Option<PathBuf> {
    let prefix = PathBuf::from(DEBUGGER_INSTALL_PATH);
    // walk the dir where we install lombok
    read_dir(&prefix)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                // get the most recently created jar file
                .filter(|path| {
                    path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jar")
                })
                .filter_map(|path| {
                    let created_time = metadata(&path).and_then(|meta| meta.created()).ok()?;
                    Some((path, created_time))
                })
                .max_by_key(|&(_, time)| time)
                .map(|(path, _)| path)
        })
        .ok()
        .flatten()
}

pub struct Debugger {
    lsp: LspWrapper,
    plugin_path: Option<PathBuf>,
}

impl Debugger {
    pub fn new(lsp: LspWrapper) -> Debugger {
        Debugger {
            plugin_path: None,
            lsp,
        }
    }

    pub fn loaded(&self) -> bool {
        self.plugin_path.is_some()
    }

    pub fn get_or_download(
        &mut self,
        language_server_id: &LanguageServerId,
        configuration: &Option<Value>,
    ) -> zed::Result<PathBuf> {
        let local = find_latest_local_debugger();

        match should_use_local_or_download(configuration, local, "debugger")? {
            Some(path) => {
                self.plugin_path = Some(path.clone());
                Ok(path)
            }
            None => self.get_or_download_fork(language_server_id),
        }
    }

    fn get_or_download_fork(
        &mut self,
        _language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let prefix = "debugger";
        let artifact = "com.microsoft.java.debug.plugin";
        let latest_version = "0.53.2";
        let jar_name = format!("{artifact}-{latest_version}.jar");
        let jar_path = PathBuf::from(prefix).join(&jar_name);

        if let Some(path) = &self.plugin_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
            && path.ends_with(jar_name)
        {
            return Ok(path.clone());
        }

        create_path_if_not_exists(prefix)?;

        download_file(
            JAVA_DEBUG_PLUGIN_FORK_URL,
            &path_to_string(jar_path.clone())?,
            DownloadedFileType::Uncompressed,
        )
        .map_err(|err| {
            format!(
                "Failed to download java-debug fork from {}: {err}",
                JAVA_DEBUG_PLUGIN_FORK_URL
            )
        })?;

        self.plugin_path = Some(jar_path.clone());
        Ok(jar_path)
    }

    #[allow(unused)]
    fn get_or_download_latest_official(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let prefix = "debugger";

        if let Some(path) = &self.plugin_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let res = fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url(MAVEN_METADATA_URL)
                .build()?,
        );

        // Maven loves to be down, trying to resolve it gracefully
        if let Err(err) = &res {
            if !fs::metadata(prefix).is_ok_and(|stat| stat.is_dir()) {
                return Err(err.to_owned());
            }

            println!(
                "Could not fetch debugger: {}\nFalling back to local version.",
                err
            );

            let exists = fs::read_dir(prefix)
                .ok()
                .and_then(|dir| dir.last().map(|v| v.ok()))
                .flatten();

            if let Some(file) = exists {
                if !file.metadata().is_ok_and(|stat| stat.is_file()) {
                    return Err(err.to_owned());
                }

                if !file
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".jar"))
                {
                    return Err(err.to_owned());
                }

                let jar_path = PathBuf::from(prefix).join(file.file_name());
                self.plugin_path = Some(jar_path.clone());

                return Ok(jar_path);
            }
        }

        let xml = String::from_utf8(res?.body).map_err(|err| {
            format!("could not get string from maven metadata response body: {err}")
        })?;

        let start_tag = "<latest>";
        let end_tag = "</latest>";

        let latest_version = xml
            .split_once(start_tag)
            .and_then(|(_, rest)| rest.split_once(end_tag))
            .map(|(content, _)| content.trim())
            .ok_or(format!("Failed to parse maven-metadata.xml response {xml}"))?;

        let artifact = "com.microsoft.java.debug.plugin";

        let jar_name = format!("{artifact}-{latest_version}.jar");
        let jar_path = PathBuf::from(prefix).join(&jar_name);

        if !fs::metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
            if let Err(err) = fs::remove_dir_all(prefix) {
                println!("failed to remove directory entry: {err}");
            }

            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            create_path_if_not_exists(prefix)?;

            let url = format!(
                "https://repo1.maven.org/maven2/com/microsoft/java/{artifact}/{latest_version}/{jar_name}"
            );

            download_file(
                url.as_str(),
                &path_to_string(&jar_path)?,
                DownloadedFileType::Uncompressed,
            )
            .map_err(|err| format!("Failed to download {url} {err}"))?;
        }

        self.plugin_path = Some(jar_path.clone());
        Ok(jar_path)
    }

    pub fn start_session(&self) -> zed::Result<TcpArgumentsTemplate> {
        let port = self.lsp.get()?.request::<u16>(
            "workspace/executeCommand",
            json!({ "command": "vscode.java.startDebugSession" }),
        )?;

        Ok(TcpArgumentsTemplate {
            host: None,
            port: Some(port),
            timeout: None,
        })
    }

    pub fn inject_config(&self, worktree: &Worktree, config_string: String) -> zed::Result<String> {
        let config: Value = serde_json::from_str(&config_string)
            .map_err(|err| format!("Failed to parse debug config {err}"))?;

        if config
            .get("request")
            .and_then(Value::as_str)
            .is_some_and(|req| req != "launch")
        {
            return Ok(config_string);
        }

        let mut config = serde_json::from_value::<JavaDebugLaunchConfig>(config)
            .map_err(|err| format!("Failed to parse java debug config {err}"))?;

        let workspace_folder = worktree.root_path();

        let (main_class, project_name) = {
            let arguments = [config.main_class.clone(), config.project_name.clone()]
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<String>>();

            let entries = self
                .lsp
                .get()?
                .resolve_main_class(arguments)?
                .into_iter()
                .filter(|entry| {
                    config
                        .main_class
                        .as_ref()
                        .map(|class| &entry.main_class == class)
                        .unwrap_or(true)
                })
                .filter(|entry| {
                    config
                        .project_name
                        .as_ref()
                        .map(|class| &entry.project_name == class)
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>();

            if entries.len() > 1 {
                return Err("Project have multiple entry points, you must explicitly specify \"mainClass\" or \"projectName\"".to_owned());
            }

            match entries.first() {
                None => (config.main_class, config.project_name),
                Some(entry) => (
                    Some(entry.main_class.to_owned()),
                    Some(entry.project_name.to_owned()),
                ),
            }
        };

        let mut classpaths = config.class_paths.unwrap_or(vec![AUTO_SCOPE.to_string()]);

        if classpaths
            .iter()
            .any(|class| SCOPES.contains(&class.as_str()))
        {
            // https://github.com/microsoft/vscode-java-debug/blob/main/src/configurationProvider.ts#L518
            let scope = {
                if classpaths.iter().any(|class| class == TEST_SCOPE) {
                    Some("test".to_string())
                } else if classpaths.iter().any(|class| class == AUTO_SCOPE) {
                    None
                } else if classpaths.iter().any(|class| class == RUNTIME_SCOPE) {
                    Some("runtime".to_string())
                } else {
                    None
                }
            };

            let arguments = vec![main_class.clone(), project_name.clone(), scope.clone()];

            let result = self.lsp.get()?.resolve_class_path(arguments)?;

            for resolved in result {
                classpaths.extend(resolved);
            }
        }

        classpaths.retain(|class| !SCOPES.contains(&class.as_str()));
        classpaths.dedup();

        config.class_paths = Some(classpaths);

        config.main_class = main_class;
        config.project_name = project_name;

        config.cwd = config.cwd.or(Some(workspace_folder.to_string()));

        let config = serde_json::to_string(&config)
            .map_err(|err| format!("Failed to stringify debug config {err}"))?
            .replace("${workspaceFolder}", &workspace_folder);

        Ok(config)
    }

    pub fn inject_plugin_into_options(
        &self,
        initialization_options: Option<Value>,
    ) -> zed::Result<Value> {
        let current_dir = get_curr_dir()?;

        let canonical_path = Value::String(
            current_dir
                .join(
                    self.plugin_path
                        .as_ref()
                        .ok_or("Debugger is not loaded yet")?,
                )
                .to_string_lossy()
                .to_string(),
        );

        match initialization_options {
            None => Ok(json!({
                "bundles": [canonical_path]
            })),
            Some(options) => {
                let mut options = options.clone();

                let mut bundles = options
                    .get_mut("bundles")
                    .unwrap_or(&mut Value::Array(vec![]))
                    .take();

                let bundles_vec = bundles
                    .as_array_mut()
                    .ok_or("Invalid initialization_options format")?;

                if !bundles_vec.contains(&canonical_path) {
                    bundles_vec.push(canonical_path);
                }

                options["bundles"] = bundles;

                Ok(options)
            }
        }
    }
}
