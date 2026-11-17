use std::{collections::HashMap, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus,
    TcpArgumentsTemplate, Worktree, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    serde_json::{self, Value, json},
    set_language_server_installation_status,
};

use crate::{
    config::get_java_debug_jar,
    downloadable::Downloadable,
    lsp,
    util::{
        ArgsStringOrList, create_path_if_not_exists, get_curr_dir, path_to_file_uri,
        path_to_string, record_successful_update_check, should_use_local_or_download,
        update_check_path,
    },
};

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct JavaDebugLaunchConfig {
    request: String,
    #[serde(skip_serializing)]
    main_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    main_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<ArgsStringOrList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vm_args: Option<ArgsStringOrList>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MainMethod {
    main_class: String,
    project_name: Option<String>,
}

impl JavaDebugLaunchConfig {
    fn prepare_main_file(
        &mut self,
        mut execute_command: impl FnMut(&str, Value) -> zed::Result<Value>,
    ) -> zed::Result<bool> {
        let Some(main_file) = &self.main_file else {
            return Ok(false);
        };

        let response = execute_command(
            "vscode.java.resolveMainMethod",
            json!([path_to_file_uri(main_file)]),
        )
        .map_err(|err| format!("Failed to resolve main method in {main_file}: {err}"))?;
        let entries: Vec<MainMethod> = serde_json::from_value(response)
            .map_err(|err| format!("Invalid main method response for {main_file}: {err}"))?;
        let mut entries = entries.into_iter().filter(|entry| {
            self.main_class.as_ref().is_none_or(|main_class| {
                main_class == &entry.main_class
                    || (!main_class.contains('.')
                        && entry.main_class.rsplit('.').next() == Some(main_class.as_str()))
            }) && self
                .project_name
                .as_ref()
                .is_none_or(|project_name| Some(project_name) == entry.project_name.as_ref())
        });
        let entry = entries
            .next()
            .ok_or_else(|| format!("No matching Java main method found in {main_file}"))?;
        if entries.next().is_some() {
            return Err(format!(
                "Multiple Java main methods found in {main_file}; specify mainClass and projectName in .zed/debug.json"
            ));
        }

        let params = json!({
            "mainClass": entry.main_class,
            "projectName": entry.project_name,
            "isFullBuild": false,
        });
        let status = execute_command("vscode.java.buildWorkspace", json!([params.to_string()]))
            .map_err(|err| format!("Failed to build Java workspace: {err}"))?;
        if status.as_u64() != Some(BUILD_SUCCEEDED) {
            return Err(format!(
                "Java build did not succeed ({status}); fix compilation errors or retry before debugging"
            ));
        }

        self.main_class = Some(entry.main_class);
        self.project_name = entry.project_name;
        Ok(true)
    }
}

const TEST_SCOPE: &str = "$Test";
const AUTO_SCOPE: &str = "$Auto";
const RUNTIME_SCOPE: &str = "$Runtime";

const SCOPES: [&str; 3] = [TEST_SCOPE, AUTO_SCOPE, RUNTIME_SCOPE];

const BUILD_SUCCEEDED: u64 = 1;

const DEBUGGER_INSTALL_PATH: &str = "debugger";

const JAVA_DEBUG_PLUGIN_FORK_URL: &str = "https://github.com/zed-industries/java-debug/releases/download/0.53.2/com.microsoft.java.debug.plugin-0.53.2.jar";

const MAVEN_METADATA_URL: &str = "https://repo1.maven.org/maven2/com/microsoft/java/com.microsoft.java.debug.plugin/maven-metadata.xml";

fn find_latest_local_debugger() -> Option<PathBuf> {
    let prefix = get_curr_dir().ok()?.join(DEBUGGER_INSTALL_PATH);
    fs::read_dir(&prefix)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("jar")
                })
                .filter_map(|path| {
                    let created_time = fs::metadata(&path).and_then(|meta| meta.created()).ok()?;
                    Some((path, created_time))
                })
                .max_by_key(|&(_, time)| time)
                .map(|(path, _)| path)
        })
        .ok()
        .flatten()
}

pub struct Debugger {
    plugin_path: Option<PathBuf>,
}

impl Debugger {
    pub fn new() -> Debugger {
        Debugger { plugin_path: None }
    }

    fn get_or_download_fork(
        &mut self,
        _language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let prefix = get_curr_dir()
            .map_err(|err| format!("Failed to get extension directory for debugger: {err}"))?
            .join(DEBUGGER_INSTALL_PATH);
        let artifact = "com.microsoft.java.debug.plugin";
        let latest_version = "0.53.2";
        let jar_name = format!("{artifact}-{latest_version}.jar");
        let jar_path = prefix.join(&jar_name);

        if let Some(path) = &self.plugin_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
            && path.ends_with(jar_name)
        {
            return Ok(path.clone());
        }

        if fs::metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
            self.plugin_path = Some(jar_path.clone());
            return Ok(jar_path);
        }

        create_path_if_not_exists(&prefix)
            .map_err(|err| format!("Failed to create debugger directory '{prefix:?}': {err}"))?;

        download_file(
            JAVA_DEBUG_PLUGIN_FORK_URL,
            &path_to_string(jar_path.clone())
                .map_err(|err| format!("Invalid debugger jar path {jar_path:?}: {err}"))?,
            DownloadedFileType::Uncompressed,
        )
        .map_err(|err| {
            format!("Failed to download java-debug fork from {JAVA_DEBUG_PLUGIN_FORK_URL}: {err}")
        })?;

        let _ = record_successful_update_check(
            &update_check_path(DEBUGGER_INSTALL_PATH),
            latest_version,
        );

        self.plugin_path = Some(jar_path.clone());
        Ok(jar_path)
    }

    #[allow(unused)]
    fn get_or_download_latest_official(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let prefix = get_curr_dir()
            .map_err(|err| format!("Failed to get extension directory for debugger: {err}"))?
            .join(DEBUGGER_INSTALL_PATH);

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
            if !fs::metadata(&prefix).is_ok_and(|stat| stat.is_dir()) {
                return Err(err.to_owned());
            }

            println!("Could not fetch debugger: {err}\nFalling back to local version.");

            let exists = fs::read_dir(&prefix)
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

                let jar_path = prefix.join(file.file_name());
                self.plugin_path = Some(jar_path.clone());

                return Ok(jar_path);
            }
        }

        let xml = String::from_utf8(res?.body).map_err(|err| {
            format!("Failed to get string from Maven metadata response body: {err}")
        })?;

        let start_tag = "<latest>";
        let end_tag = "</latest>";

        let latest_version = xml
            .split_once(start_tag)
            .and_then(|(_, rest)| rest.split_once(end_tag))
            .map(|(content, _)| content.trim())
            .ok_or(format!(
                "Failed to parse maven-metadata.xml response: {xml}"
            ))?;

        let artifact = "com.microsoft.java.debug.plugin";

        let jar_name = format!("{artifact}-{latest_version}.jar");
        let jar_path = prefix.join(&jar_name);

        if !fs::metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
            if let Err(err) = fs::remove_dir_all(&prefix) {
                println!("failed to remove directory entry: {err}");
            }

            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            create_path_if_not_exists(&prefix)?;

            let url = format!(
                "https://repo1.maven.org/maven2/com/microsoft/java/{artifact}/{latest_version}/{jar_name}"
            );

            download_file(
                url.as_str(),
                &path_to_string(&jar_path)?,
                DownloadedFileType::Uncompressed,
            )
            .map_err(|err| format!("Failed to download {url}: {err}"))?;

            let _ = record_successful_update_check(
                &update_check_path(DEBUGGER_INSTALL_PATH),
                latest_version,
            );
        }

        self.plugin_path = Some(jar_path.clone());
        Ok(jar_path)
    }

    pub fn start_session(&self, workspace: &str) -> zed::Result<TcpArgumentsTemplate> {
        let port = lsp::request::<u16>(
            workspace,
            "workspace/executeCommand",
            json!({ "command": "vscode.java.startDebugSession" }),
        )
        .map_err(|err| format!("Failed to start debug session via LSP: {err}"))?;

        Ok(TcpArgumentsTemplate {
            host: None,
            port: Some(port),
            timeout: None,
        })
    }

    pub fn inject_config(&self, worktree: &Worktree, config_string: String) -> zed::Result<String> {
        let config: Value = serde_json::from_str(&config_string)
            .map_err(|err| format!("Failed to parse debug config: {err}"))?;

        if config
            .get("request")
            .and_then(Value::as_str)
            .is_some_and(|req| req != "launch")
        {
            return Ok(config_string);
        }

        let mut config = serde_json::from_value::<JavaDebugLaunchConfig>(config)
            .map_err(|err| format!("Failed to parse Java debug config: {err}"))?;

        let workspace_folder = worktree.root_path();

        let file_launch = config.prepare_main_file(|command, arguments| {
            lsp::request(
                &workspace_folder,
                "workspace/executeCommand",
                json!({ "command": command, "arguments": arguments }),
            )
        })?;

        let (main_class, project_name) = if file_launch {
            (config.main_class.take(), config.project_name.take())
        } else {
            let arguments = [config.main_class.clone(), config.project_name.clone()]
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<String>>();

            let entries = lsp::resolve_main_class(&workspace_folder, arguments)
                .map_err(|err| format!("Failed to resolve main class: {err}"))?
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

            let result = lsp::resolve_class_path(&workspace_folder, arguments)
                .map_err(|err| format!("Failed to resolve classpath: {err}"))?;

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
            .map_err(|err| format!("Failed to stringify debug config: {err}"))?
            .replace("${workspaceFolder}", &workspace_folder);

        Ok(config)
    }

    pub fn plugin_path(&self) -> Option<&PathBuf> {
        self.plugin_path.as_ref()
    }

    pub fn inject_plugin_into_options(
        &self,
        initialization_options: Option<Value>,
    ) -> zed::Result<Value> {
        let canonical_path = Value::String(
            self.plugin_path
                .as_ref()
                .ok_or("Debugger plugin path not set")?
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

impl Downloadable for Debugger {
    const INSTALL_PATH: &'static str = DEBUGGER_INSTALL_PATH;

    fn find_local(&self) -> Option<PathBuf> {
        find_latest_local_debugger()
    }

    fn loaded(&self) -> bool {
        self.plugin_path.is_some()
    }

    fn fetch_latest_version(&self, _worktree: &Worktree) -> zed::Result<String> {
        Ok("0.53.2".to_string())
    }

    fn download(
        &mut self,
        _version: &str,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        self.get_or_download_fork(language_server_id)
    }

    fn get_or_download(
        &mut self,
        language_server_id: &LanguageServerId,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        if let Some(jar_path) = self.user_configured_path(configuration, worktree) {
            let path = PathBuf::from(jar_path);
            self.plugin_path = Some(path.clone());
            return Ok(path);
        }

        if let Some(path) = should_use_local_or_download(
            configuration,
            self.find_local(),
            Self::INSTALL_PATH,
            &self.update_check_path(),
        )
        .map_err(|err| format!("Failed to resolve debugger installation: {err}"))?
        {
            self.plugin_path = Some(path.clone());
            return Ok(path);
        }

        self.get_or_download_fork(language_server_id)
    }

    fn user_configured_path(
        &self,
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> Option<String> {
        get_java_debug_jar(configuration, worktree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepare_file_launch(
        config: Value,
        main_methods: Value,
        build_status: Value,
    ) -> (
        JavaDebugLaunchConfig,
        zed::Result<bool>,
        Vec<(String, Value)>,
    ) {
        let mut config: JavaDebugLaunchConfig = serde_json::from_value(config).unwrap();
        let mut requests = Vec::new();
        let result = config.prepare_main_file(|command, arguments| {
            requests.push((command.to_string(), arguments));
            match command {
                "vscode.java.resolveMainMethod" => Ok(main_methods.clone()),
                "vscode.java.buildWorkspace" => Ok(build_status.clone()),
                _ => panic!("Unexpected command: {command}"),
            }
        });

        (config, result, requests)
    }

    #[test]
    fn file_launch_resolves_the_source_project_and_builds_before_launch() {
        let (config, result, requests) = prepare_file_launch(
            json!({
                "request": "launch",
                "mainFile": "/workspace/module b/src/主类 #1%.java",
                "env": { "APP_MODE": "development" },
                "args": ["hello"],
            }),
            json!([{
                "mainClass": "example.Main",
                "projectName": "module-b",
                "range": {
                    "start": { "line": 3, "character": 23 },
                    "end": { "line": 3, "character": 27 },
                },
            }]),
            json!(1),
        );

        assert!(result.unwrap());
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0],
            (
                "vscode.java.resolveMainMethod".into(),
                json!(["file:///workspace/module%20b/src/%E4%B8%BB%E7%B1%BB%20%231%25.java"]),
            )
        );
        assert_eq!(requests[1].0, "vscode.java.buildWorkspace");
        assert_eq!(
            serde_json::from_str::<Value>(requests[1].1[0].as_str().unwrap()).unwrap(),
            json!({
                "mainClass": "example.Main",
                "projectName": "module-b",
                "isFullBuild": false,
            })
        );
        let config = serde_json::to_value(config).unwrap();
        assert_eq!(config["mainClass"], "example.Main");
        assert_eq!(config["projectName"], "module-b");
        assert_eq!(config["env"], json!({ "APP_MODE": "development" }));
        assert_eq!(config["args"], "hello");
        assert!(config.get("mainFile").is_none());
    }

    #[test]
    fn manual_launch_without_main_file_skips_file_resolution_and_build() {
        let mut config: JavaDebugLaunchConfig = serde_json::from_value(json!({
            "request": "launch",
            "mainClass": "example.Main",
            "projectName": "module-a",
        }))
        .unwrap();

        assert!(
            !config
                .prepare_main_file(|command, _| panic!("Unexpected command: {command}"))
                .unwrap()
        );
        assert_eq!(config.main_class.as_deref(), Some("example.Main"));
        assert_eq!(config.project_name.as_deref(), Some("module-a"));
    }

    #[test]
    fn missing_main_method_does_not_build_or_fall_back_to_another_project() {
        for (filters, entries) in [
            (json!({}), json!([])),
            (
                json!({ "mainClass": "example.Missing" }),
                json!([{ "mainClass": "example.Main", "projectName": "module-a" }]),
            ),
            (
                json!({ "projectName": "module-b" }),
                json!([{ "mainClass": "example.Main", "projectName": "module-a" }]),
            ),
        ] {
            let mut config = json!({
                "request": "launch",
                "mainFile": "/workspace/Main.java",
            });
            config
                .as_object_mut()
                .unwrap()
                .extend(filters.as_object().unwrap().clone());
            let (_, result, requests) = prepare_file_launch(config, entries, json!(1));

            assert!(result.unwrap_err().contains("No matching Java main method"));
            assert_eq!(requests.len(), 1);
        }
    }

    #[test]
    fn multiple_main_methods_require_selection_before_building() {
        let (_, result, requests) = prepare_file_launch(
            json!({ "request": "launch", "mainFile": "/workspace/Main.java" }),
            json!([
                { "mainClass": "example.Main", "projectName": "module-a" },
                { "mainClass": "example.Other", "projectName": "module-a" },
            ]),
            json!(1),
        );

        assert!(result.unwrap_err().contains("Multiple Java main methods"));
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn explicit_class_and_project_select_the_matching_file_entry() {
        let (config, result, requests) = prepare_file_launch(
            json!({
                "request": "launch",
                "mainFile": "/workspace/Main.java",
                "mainClass": "example.Main",
                "projectName": "module-b",
            }),
            json!([
                { "mainClass": "example.Main", "projectName": "module-a" },
                { "mainClass": "example.Other", "projectName": "module-b" },
                { "mainClass": "example.Main", "projectName": "module-b" },
            ]),
            json!(1),
        );

        assert!(result.unwrap());
        assert_eq!(config.main_class.as_deref(), Some("example.Main"));
        assert_eq!(config.project_name.as_deref(), Some("module-b"));
        assert_eq!(
            serde_json::from_str::<Value>(requests[1].1[0].as_str().unwrap()).unwrap()["projectName"],
            "module-b"
        );
    }

    #[test]
    fn gutter_class_selects_the_clicked_main_in_the_same_file() {
        for package in ["example.", ""] {
            let expected_class = format!("{package}Other");
            let (config, result, requests) = prepare_file_launch(
                json!({
                    "request": "launch",
                    "mainFile": "/workspace/Main.java",
                    "mainClass": "Other",
                }),
                json!([
                    { "mainClass": format!("{package}Main"), "projectName": "module-a" },
                    { "mainClass": expected_class, "projectName": "module-a" },
                ]),
                json!(1),
            );

            assert!(result.unwrap());
            assert_eq!(config.main_class.as_deref(), Some(expected_class.as_str()));
            assert_eq!(config.project_name.as_deref(), Some("module-a"));
            assert_eq!(
                serde_json::from_str::<Value>(requests[1].1[0].as_str().unwrap()).unwrap()["mainClass"],
                expected_class
            );
        }
    }

    #[test]
    fn class_hints_do_not_match_suffixes_or_other_packages() {
        for (main_class, entry_class) in [
            ("Main", "example.OtherMain"),
            ("another.Main", "example.Main"),
        ] {
            let (_, result, requests) = prepare_file_launch(
                json!({
                    "request": "launch",
                    "mainFile": "/workspace/OtherMain.java",
                    "mainClass": main_class,
                }),
                json!([{ "mainClass": entry_class, "projectName": "module-a" }]),
                json!(1),
            );

            assert!(result.unwrap_err().contains("No matching Java main method"));
            assert_eq!(requests.len(), 1);
        }
    }

    #[test]
    fn simple_class_hint_does_not_hide_ambiguous_projects() {
        let (_, result, requests) = prepare_file_launch(
            json!({
                "request": "launch",
                "mainFile": "/workspace/Main.java",
                "mainClass": "Main",
            }),
            json!([
                { "mainClass": "example.Main", "projectName": "module-a" },
                { "mainClass": "example.Main", "projectName": "module-b" },
            ]),
            json!(1),
        );

        assert!(result.unwrap_err().contains("Multiple Java main methods"));
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn file_resolution_preserves_an_unnamed_project() {
        let (config, result, requests) = prepare_file_launch(
            json!({ "request": "launch", "mainFile": "/workspace/Main.java" }),
            json!([{ "mainClass": "Main", "projectName": null }]),
            json!(1),
        );

        assert!(result.unwrap());
        assert_eq!(config.main_class.as_deref(), Some("Main"));
        assert!(config.project_name.is_none());
        assert!(
            serde_json::from_str::<Value>(requests[1].1[0].as_str().unwrap()).unwrap()["projectName"]
                .is_null()
        );
    }

    #[test]
    fn unsuccessful_or_unknown_build_status_stops_the_launch() {
        for status in [
            json!(0),
            json!(2),
            json!(3),
            json!(100),
            json!("SUCCEED"),
            json!("UNKNOWN"),
            Value::Null,
        ] {
            let (config, result, requests) = prepare_file_launch(
                json!({ "request": "launch", "mainFile": "/workspace/Main.java" }),
                json!([{ "mainClass": "example.Main", "projectName": "module-a" }]),
                status,
            );

            assert!(result.unwrap_err().contains("Java build did not succeed"));
            assert!(config.main_class.is_none());
            assert_eq!(requests.len(), 2);
        }
    }

    #[test]
    fn file_resolution_and_build_request_failures_stop_the_launch() {
        for failed_command in [
            "vscode.java.resolveMainMethod",
            "vscode.java.buildWorkspace",
        ] {
            let mut config: JavaDebugLaunchConfig = serde_json::from_value(json!({
                "request": "launch",
                "mainFile": "/workspace/Main.java",
            }))
            .unwrap();
            let mut requests = Vec::new();
            let result = config.prepare_main_file(|command, _| {
                requests.push(command.to_string());
                if command == failed_command {
                    return Err("LSP request failed".into());
                }
                Ok(json!([{ "mainClass": "example.Main", "projectName": "module-a" }]))
            });

            assert!(result.unwrap_err().contains("LSP request failed"));
            assert_eq!(requests.last().unwrap(), failed_command);
            assert!(config.main_class.is_none());
        }
    }

    #[test]
    fn invalid_main_method_responses_stop_before_building() {
        for response in [Value::Null, json!({}), json!([{ "mainClass": 42 }])] {
            let (_, result, requests) = prepare_file_launch(
                json!({ "request": "launch", "mainFile": "/workspace/Main.java" }),
                response,
                json!(1),
            );

            assert!(result.unwrap_err().contains("Invalid main method response"));
            assert_eq!(requests.len(), 1);
        }
    }

    fn injected_bundle(plugin_path: PathBuf) -> String {
        let debugger = Debugger {
            plugin_path: Some(plugin_path),
        };
        debugger
            .inject_plugin_into_options(None)
            .unwrap()
            .pointer("/bundles/0")
            .and_then(Value::as_str)
            .unwrap()
            .to_string()
    }

    #[test]
    fn managed_debugger_bundle_is_resolved_against_extension_directory() {
        let plugin_path = get_curr_dir().unwrap().join("debugger/java-debug.jar");

        assert_eq!(
            injected_bundle(plugin_path),
            get_curr_dir()
                .unwrap()
                .join("debugger/java-debug.jar")
                .to_string_lossy()
        );
    }

    #[test]
    fn external_debugger_bundle_is_preserved() {
        let plugin_path = PathBuf::from(r"C:\tools\java-debug.jar");

        assert_eq!(
            injected_bundle(plugin_path),
            r"C:\tools\java-debug.jar".to_string()
        );
    }
}
