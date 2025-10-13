mod debugger;
mod lsp;
use std::{
    collections::BTreeSet,
    env,
    fs::{self, create_dir},
    path::{Path, PathBuf},
    str::FromStr,
};

use regex::Regex;
use sha1::{Digest, Sha1};
use zed_extension_api::{
    self as zed, CodeLabel, CodeLabelSpan, DebugAdapterBinary, DebugTaskDefinition,
    DownloadedFileType, Extension, LanguageServerId, LanguageServerInstallationStatus, Os,
    StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest, Worktree,
    current_platform, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    lsp::{Completion, CompletionKind},
    make_file_executable,
    process::Command,
    register_extension,
    serde_json::{self, Value, json},
    set_language_server_installation_status,
    settings::LspSettings,
};

use crate::{debugger::Debugger, lsp::LspWrapper};

const PROXY_FILE: &str = include_str!("proxy.mjs");
const DEBUG_ADAPTER_NAME: &str = "Java";
const PATH_TO_STR_ERROR: &str = "failed to convert path to string";
const JDTLS_INSTALL_PATH: &str = "jdtls";
const LOMBOK_INSTALL_PATH: &str = "lombok";

struct Java {
    cached_binary_path: Option<PathBuf>,
    cached_lombok_path: Option<PathBuf>,
    integrations: Option<(LspWrapper, Debugger)>,
}

impl Java {
    fn lsp(&mut self) -> zed::Result<&LspWrapper> {
        self.integrations
            .as_ref()
            .ok_or("Lsp client is not initialized yet".to_owned())
            .map(|v| &v.0)
    }

    fn debugger(&mut self) -> zed::Result<&mut Debugger> {
        self.integrations
            .as_mut()
            .ok_or("Lsp client is not initialized yet".to_owned())
            .map(|v| &mut v.1)
    }

    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        // Initialize lsp client and debugger

        if self.integrations.is_none() {
            let lsp = LspWrapper::new(worktree.root_path());
            let debugger = Debugger::new(lsp.clone());

            self.integrations = Some((lsp, debugger));
        }

        // Use cached path if exists

        if let Some(path) = &self.cached_binary_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        // Use $PATH if binary is in it

        let (platform, _) = current_platform();
        let binary_name = match platform {
            Os::Windows => "jdtls.bat",
            _ => "jdtls",
        };

        // Check for latest version

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        match try_to_fetch_and_install_latest_jdtls(binary_name, language_server_id) {
            Ok(path) => {
                self.cached_binary_path = Some(path.clone());
                Ok(path)
            }
            Err(e) => {
                if let Some(local_version) = find_latest_local_jdtls(binary_name) {
                    self.cached_binary_path = Some(local_version.clone());
                    Ok(local_version)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn lombok_jar_path(&mut self, language_server_id: &LanguageServerId) -> zed::Result<PathBuf> {
        if let Some(path) = &self.cached_lombok_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        match try_to_fetch_and_install_latest_lombok(language_server_id) {
            Ok(path) => {
                self.cached_lombok_path = Some(path.clone());
                return Ok(path);
            }
            Err(e) => {
                if let Some(local_version) = find_latest_local_lombok() {
                    self.cached_lombok_path = Some(local_version.clone());
                    Ok(local_version)
                } else {
                    Err(e)
                }
            }
        }
    }
}

fn try_to_fetch_and_install_latest_jdtls(
    binary_name: &str,
    language_server_id: &LanguageServerId,
) -> zed::Result<PathBuf> {
    // Yeah, this part's all pretty terrible...
    // Note to self: make it good eventually
    let downloads_html = String::from_utf8(
        fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url("https://download.eclipse.org/jdtls/milestones/")
                .build()?,
        )
        .map_err(|err| format!("failed to get available versions: {err}"))?
        .body,
    )
    .map_err(|err| format!("could not get string from downloads page response body: {err}"))?;
    let mut versions = BTreeSet::new();
    let mut number_buffer = String::new();
    let mut version_buffer: (Option<u32>, Option<u32>, Option<u32>) = (None, None, None);

    for char in downloads_html.chars() {
        if char.is_numeric() {
            number_buffer.push(char);
        } else if char == '.' {
            if version_buffer.0.is_none() && !number_buffer.is_empty() {
                version_buffer.0 = Some(
                    number_buffer
                        .parse()
                        .map_err(|err| format!("could not parse number buffer: {err}"))?,
                );
            } else if version_buffer.1.is_none() && !number_buffer.is_empty() {
                version_buffer.1 = Some(
                    number_buffer
                        .parse()
                        .map_err(|err| format!("could not parse number buffer: {err}"))?,
                );
            } else {
                version_buffer = (None, None, None);
            }

            number_buffer.clear();
        } else {
            if version_buffer.0.is_some()
                && version_buffer.1.is_some()
                && version_buffer.2.is_none()
            {
                versions.insert((
                    version_buffer.0.ok_or("no major version number")?,
                    version_buffer.1.ok_or("no minor version number")?,
                    number_buffer
                        .parse::<u32>()
                        .map_err(|err| format!("could not parse number buffer: {err}"))?,
                ));
            }

            number_buffer.clear();
            version_buffer = (None, None, None);
        }
    }

    let (major, minor, patch) = versions.last().ok_or("no available versions")?;
    let latest_version = format!("{major}.{minor}.{patch}");
    let latest_version_build = String::from_utf8(
        fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url(format!(
                    "https://download.eclipse.org/jdtls/milestones/{latest_version}/latest.txt"
                ))
                .build()?,
        )
        .map_err(|err| format!("failed to get latest version's build: {err}"))?
        .body,
    )
    .map_err(|err| {
        format!("attempt to get latest version's build resulted in a malformed response: {err}")
    })?;
    let latest_version_build = latest_version_build.trim_end();
    let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
    // Exclude ".tar.gz"
    let build_directory = &latest_version_build[..latest_version_build.len() - 7];
    let build_path = prefix.join(build_directory);
    let binary_path = build_path.join("bin").join(binary_name);

    // If latest version isn't installed,
    if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
        // then download it...

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        download_file(
            &format!(
                "https://www.eclipse.org/downloads/download.php?file=/jdtls/milestones/{latest_version}/{latest_version_build}",
            ),
            build_path.to_str().ok_or(PATH_TO_STR_ERROR)?,
            DownloadedFileType::GzipTar,
        )?;
        make_file_executable(binary_path.to_str().ok_or(PATH_TO_STR_ERROR)?)?;

        // ...and delete other versions

        // This step is expected to fail sometimes, and since we don't know
        // how to fix it yet, we just carry on so the user doesn't have to
        // restart the language server.
        match fs::read_dir(prefix) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            if entry.file_name().to_str() != Some(build_directory)
                                && let Err(err) = fs::remove_dir_all(entry.path())
                            {
                                println!("failed to remove directory entry: {err}");
                            }
                        }
                        Err(err) => println!("failed to load directory entry: {err}"),
                    }
                }
            }
            Err(err) => println!("failed to list prefix directory: {err}"),
        }
    }

    // else use it
    Ok(binary_path)
}

fn find_latest_local_jdtls(binary_name: &str) -> Option<PathBuf> {
    let prefix = PathBuf::from(JDTLS_INSTALL_PATH);
    // walk the dir where we install jdtls
    fs::read_dir(&prefix)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                // get the most recently created subdirectory
                .filter_map(|path| {
                    let created_time = fs::metadata(&path).and_then(|meta| meta.created()).ok()?;
                    Some((path, created_time))
                })
                .max_by_key(|&(_, time)| time)
                // point at where the binary should be
                .map(|(path, _)| path.join("bin").join(binary_name))
        })
        .ok()
        .flatten()
}

fn try_to_fetch_and_install_latest_lombok(
    language_server_id: &LanguageServerId,
) -> zed::Result<PathBuf> {
    set_language_server_installation_status(
        language_server_id,
        &LanguageServerInstallationStatus::CheckingForUpdate,
    );

    let tags_response_body = serde_json::from_slice::<Value>(
        &fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url("https://api.github.com/repos/projectlombok/lombok/tags")
                .build()?,
        )
        .map_err(|err| format!("failed to fetch GitHub tags: {err}"))?
        .body,
    )
    .map_err(|err| format!("failed to deserialize GitHub tags response: {err}"))?;
    let latest_version = &tags_response_body
        .as_array()
        .and_then(|tag| {
            tag.first().and_then(|latest_tag| {
                latest_tag
                    .get("name")
                    .and_then(|tag_name| tag_name.as_str())
            })
        })
        // Exclude 'v' at beginning
        .ok_or("malformed GitHub tags response")?[1..];
    let prefix = LOMBOK_INSTALL_PATH;
    let jar_name = format!("lombok-{latest_version}.jar");
    let jar_path = Path::new(prefix).join(&jar_name);

    // If latest version isn't installed,
    if !fs::metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
        // then download it...

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );
        create_dir(prefix).map_err(|err| err.to_string())?;
        download_file(
            &format!("https://projectlombok.org/downloads/{jar_name}"),
            jar_path.to_str().ok_or(PATH_TO_STR_ERROR)?,
            DownloadedFileType::Uncompressed,
        )?;

        // ...and delete other versions

        // This step is expected to fail sometimes, and since we don't know
        // how to fix it yet, we just carry on so the user doesn't have to
        // restart the language server.
        match fs::read_dir(prefix) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => {
                            if entry.file_name().to_str() != Some(&jar_name)
                                && let Err(err) = fs::remove_dir_all(entry.path())
                            {
                                println!("failed to remove directory entry: {err}");
                            }
                        }
                        Err(err) => println!("failed to load directory entry: {err}"),
                    }
                }
            }
            Err(err) => println!("failed to list prefix directory: {err}"),
        }
    }

    // else use it
    Ok(jar_path)
}

fn find_latest_local_lombok() -> Option<PathBuf> {
    let prefix = PathBuf::from(LOMBOK_INSTALL_PATH);
    // walk the dir where we install lombok
    fs::read_dir(&prefix)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                // get the most recently created jar file
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

impl Extension for Java {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self {
            cached_binary_path: None,
            cached_lombok_path: None,
            integrations: None,
        }
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: DebugTaskDefinition,
        _user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> zed_extension_api::Result<DebugAdapterBinary, String> {
        if !self.debugger().is_ok_and(|v| v.loaded()) {
            return Err("Debugger plugin is not loaded".to_string());
        }

        if adapter_name != DEBUG_ADAPTER_NAME {
            return Err(format!(
                "Cannot create binary for adapter \"{adapter_name}\""
            ));
        }

        if self.integrations.is_some() {
            self.lsp()?.switch_workspace(worktree.root_path())?;
        }

        Ok(DebugAdapterBinary {
            command: None,
            arguments: vec![],
            cwd: Some(worktree.root_path()),
            envs: vec![],
            request_args: StartDebuggingRequestArguments {
                request: self.dap_request_kind(
                    adapter_name,
                    Value::from_str(config.config.as_str())
                        .map_err(|e| format!("Invalid JSON configuration: {e}"))?,
                )?,
                configuration: self.debugger()?.inject_config(worktree, config.config)?,
            },
            connection: Some(zed::resolve_tcp_template(
                self.debugger()?.start_session()?,
            )?),
        })
    }

    fn dap_request_kind(
        &mut self,
        adapter_name: String,
        config: Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        if adapter_name != DEBUG_ADAPTER_NAME {
            return Err(format!(
                "Cannot create binary for adapter \"{adapter_name}\""
            ));
        }

        match config.get("request") {
            Some(launch) if launch == "launch" => Ok(StartDebuggingRequestArgumentsRequest::Launch),
            Some(attach) if attach == "attach" => Ok(StartDebuggingRequestArgumentsRequest::Attach),
            Some(value) => Err(format!(
                "Unexpected value for `request` key in Java debug adapter configuration: {value:?}"
            )),
            None => {
                Err("Missing required `request` field in Java debug adapter configuration".into())
            }
        }
    }

    fn dap_config_to_scenario(
        &mut self,
        config: zed::DebugConfig,
    ) -> zed::Result<zed::DebugScenario, String> {
        if !self.debugger().is_ok_and(|v| v.loaded()) {
            return Err("Debugger plugin is not loaded".to_string());
        }

        match config.request {
            zed::DebugRequest::Attach(attach) => {
                let debug_config = if let Some(process_id) = attach.process_id {
                    json!({
                        "request": "attach",
                        "processId": process_id,
                        "stopOnEntry": config.stop_on_entry
                    })
                } else {
                    json!({
                        "request": "attach",
                        "hostName": "localhost",
                        "port": 5005,
                    })
                };

                Ok(zed::DebugScenario {
                    adapter: config.adapter,
                    build: None,
                    tcp_connection: Some(self.debugger()?.start_session()?),
                    label: "Attach to Java process".to_string(),
                    config: debug_config.to_string(),
                })
            }

            zed::DebugRequest::Launch(_launch) => {
                Err("Java Extension doesn't support launching".to_string())
            }
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        let current_dir =
            env::current_dir().map_err(|err| format!("could not get current dir: {err}"))?;

        let configuration =
            self.language_server_workspace_configuration(language_server_id, worktree)?;

        let mut env = Vec::new();

        if let Some(java_home) = get_java_home(&configuration) {
            env.push(("JAVA_HOME".to_string(), java_home));
        }

        // our proxy takes workdir, bin, argv
        let mut args = vec![
            "--input-type=module".to_string(),
            "-e".to_string(),
            PROXY_FILE.to_string(),
            path_to_string(current_dir.clone())?,
        ];

        // Add lombok as javaagent if settings.java.jdt.ls.lombokSupport.enabled is true
        let lombok_enabled = configuration
            .as_ref()
            .and_then(|configuration| {
                configuration
                    .pointer("/java/jdt/ls/lombokSupport/enabled")
                    .and_then(|enabled| enabled.as_bool())
            })
            .unwrap_or(false);

        let lombok_jvm_arg = if lombok_enabled {
            let lombok_jar_path = self.lombok_jar_path(language_server_id)?;
            let canonical_lombok_jar_path = current_dir
                .join(lombok_jar_path)
                .to_str()
                .ok_or(PATH_TO_STR_ERROR)?
                .to_string();
            Some(format!("-javaagent:{canonical_lombok_jar_path}"))
        } else {
            None
        };

        if let Some(launcher) = get_jdtls_launcher_from_path(worktree) {
            // if the user has `jdtls(.bat)` on their PATH, we use that
            args.push(launcher);
            if let Some(lombok_jvm_arg) = lombok_jvm_arg {
                args.push(format!("--jvm-arg={lombok_jvm_arg}"));
            }
        } else {
            // otherwise we launch ourselves
            args.extend(self.build_jdtls_launch_args(
                &configuration,
                language_server_id,
                worktree,
                lombok_jvm_arg.into_iter().collect(), // TODO additional jvm-args from config?
            )?);
        }

        // download debugger if not exists
        if let Err(err) = self.debugger()?.get_or_download(language_server_id) {
            println!("Failed to download debugger: {err}");
        };

        self.lsp()?.switch_workspace(worktree.root_path())?;

        Ok(zed::Command {
            command: zed::node_binary_path()?,
            args,
            env,
        })
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        if self.integrations.is_some() {
            self.lsp()?.switch_workspace(worktree.root_path())?;
        }

        let options = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.initialization_options)?;

        if self.debugger().is_ok_and(|v| v.loaded()) {
            return Ok(Some(self.debugger()?.inject_plugin_into_options(options)?));
        }

        Ok(options)
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        if let Ok(Some(settings)) = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.settings)
        {
            Ok(Some(settings))
        } else {
            self.language_server_initialization_options(language_server_id, worktree)
                .map(|init_options| {
                    init_options.and_then(|init_options| init_options.get("settings").cloned())
                })
        }
    }

    fn label_for_completion(
        &self,
        _language_server_id: &LanguageServerId,
        completion: Completion,
    ) -> Option<CodeLabel> {
        // uncomment when debugging completions
        // println!("Java completion: {completion:#?}");

        completion.kind.and_then(|kind| match kind {
            CompletionKind::Field | CompletionKind::Constant => {
                let modifiers = match kind {
                    CompletionKind::Field => "",
                    CompletionKind::Constant => "static final ",
                    _ => return None,
                };
                let property_type = completion.detail.as_ref().and_then(|detail| {
                    detail
                        .split_once(" : ")
                        .map(|(_, property_type)| format!("{property_type} "))
                })?;
                let semicolon = ";";
                let code = format!("{modifiers}{property_type}{}{semicolon}", completion.label);

                Some(CodeLabel {
                    spans: vec![
                        CodeLabelSpan::code_range(
                            modifiers.len() + property_type.len()..code.len() - semicolon.len(),
                        ),
                        CodeLabelSpan::literal(" : ", None),
                        CodeLabelSpan::code_range(
                            modifiers.len()..modifiers.len() + property_type.len(),
                        ),
                    ],
                    code,
                    filter_range: (0..completion.label.len()).into(),
                })
            }
            CompletionKind::Method => {
                let detail = completion.detail?;
                let (left, return_type) = detail
                    .split_once(" : ")
                    .map(|(left, return_type)| (left, format!("{return_type} ")))
                    .unwrap_or((&detail, "void".to_string()));
                let parameters = left
                    .find('(')
                    .map(|parameters_start| &left[parameters_start..]);
                let name_and_parameters =
                    format!("{}{}", completion.label, parameters.unwrap_or("()"));
                let braces = " {}";
                let code = format!("{return_type}{name_and_parameters}{braces}");
                let mut spans = vec![CodeLabelSpan::code_range(
                    return_type.len()..code.len() - braces.len(),
                )];

                if parameters.is_some() {
                    spans.push(CodeLabelSpan::literal(" : ", None));
                    spans.push(CodeLabelSpan::code_range(0..return_type.len()));
                } else {
                    spans.push(CodeLabelSpan::literal(" - ", None));
                    spans.push(CodeLabelSpan::literal(detail, None));
                }

                Some(CodeLabel {
                    spans,
                    code,
                    filter_range: (0..completion.label.len()).into(),
                })
            }
            CompletionKind::Class | CompletionKind::Interface | CompletionKind::Enum => {
                let keyword = match kind {
                    CompletionKind::Class => "class ",
                    CompletionKind::Interface => "interface ",
                    CompletionKind::Enum => "enum ",
                    _ => return None,
                };
                let braces = " {}";
                let code = format!("{keyword}{}{braces}", completion.label);
                let namespace = completion
                    .detail
                    .map(|detail| detail[..detail.len() - completion.label.len() - 1].to_string());
                let mut spans = vec![CodeLabelSpan::code_range(
                    keyword.len()..code.len() - braces.len(),
                )];

                if let Some(namespace) = namespace {
                    spans.push(CodeLabelSpan::literal(format!(" ({namespace})"), None));
                }

                Some(CodeLabel {
                    spans,
                    code,
                    filter_range: (0..completion.label.len()).into(),
                })
            }
            CompletionKind::Snippet => Some(CodeLabel {
                code: String::new(),
                spans: vec![CodeLabelSpan::literal(
                    format!("{} - {}", completion.label, completion.detail?),
                    None,
                )],
                filter_range: (0..completion.label.len()).into(),
            }),
            CompletionKind::Keyword | CompletionKind::Variable => Some(CodeLabel {
                spans: vec![CodeLabelSpan::code_range(0..completion.label.len())],
                filter_range: (0..completion.label.len()).into(),
                code: completion.label,
            }),
            CompletionKind::Constructor => {
                let detail = completion.detail?;
                let parameters = &detail[detail.find('(')?..];
                let braces = " {}";
                let code = format!("{}{parameters}{braces}", completion.label);

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(0..code.len() - braces.len())],
                    code,
                    filter_range: (0..completion.label.len()).into(),
                })
            }
            _ => None,
        })
    }
}

impl Java {
    fn build_jdtls_launch_args(
        &mut self,
        configuration: &Option<Value>,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
        jvm_args: Vec<String>,
    ) -> zed::Result<Vec<String>> {
        if let Some(jdtls_launcher) = get_jdtls_launcher_from_path(worktree) {
            return Ok(vec![jdtls_launcher]);
        }

        let java_executable = get_java_executable(configuration, worktree)?;
        let java_major_version = get_java_major_version(&java_executable)?;
        if java_major_version < 21 {
            // TODO this error message could be more helpful
            return Err("JDTLS requires at least Java 21".to_string());
        }

        let extension_workdir = env::current_dir().map_err(|_e| "Could not get current dir")?;

        let jdtls_launch_script_path =
            extension_workdir.join(self.language_server_binary_path(language_server_id, worktree)?);

        // TODO we might as well return the base path directly
        let jdtls_base_path = jdtls_launch_script_path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| "Could not get JDTLS base path".to_string())?;

        let shared_config_path = get_shared_config_path(&jdtls_base_path);
        let jar_path = find_equinox_launcher(&jdtls_base_path)?;
        let jdtls_data_path = get_jdtls_data_path(worktree)?;

        let mut args = vec![
            get_java_executable(configuration, worktree).and_then(path_to_string)?,
            "-Declipse.application=org.eclipse.jdt.ls.core.id1".to_string(),
            "-Dosgi.bundles.defaultStartLevel=4".to_string(),
            "-Declipse.product=org.eclipse.jdt.ls.core.product".to_string(),
            "-Dosgi.checkConfiguration=true".to_string(),
            format!(
                "-Dosgi.sharedConfiguration.area={}",
                path_to_string(shared_config_path)?
            ),
            "-Dosgi.sharedConfiguration.area.readOnly=true".to_string(),
            "-Dosgi.configuration.cascaded=true".to_string(),
            "-Xms1G".to_string(),
            "--add-modules=ALL-SYSTEM".to_string(),
            "--add-opens".to_string(),
            "java.base/java.util=ALL-UNNAMED".to_string(),
            "--add-opens".to_string(),
            "java.base/java.lang=ALL-UNNAMED".to_string(),
        ];
        args.extend(jvm_args);
        args.extend(vec![
            "-jar".to_string(),
            path_to_string(jar_path)?,
            "-data".to_string(),
            path_to_string(jdtls_data_path)?,
        ]);
        if java_major_version >= 24 {
            args.push("-Djdk.xml.maxGeneralEntitySizeLimit=0".to_string());
            args.push("-Djdk.xml.totalEntitySizeLimit=0".to_string());
        }
        Ok(args)
    }
}

fn path_to_string(path: PathBuf) -> zed::Result<String> {
    path.into_os_string()
        .into_string()
        .map_err(|_| PATH_TO_STR_ERROR.to_string())
}

fn get_jdtls_data_path(worktree: &Worktree) -> zed::Result<PathBuf> {
    // Note: the JDTLS data path is where JDTLS stores its own caches.
    // In the unlikely event we can't find the canonical OS-Level cache-path,
    // we fall back to the the extension's workdir, which may never get cleaned up.
    // In future we may want to deliberately manage caches to be able to force-clean them.

    let mut env_iter = worktree.shell_env().into_iter();
    let base_cachedir = match current_platform().0 {
        Os::Mac => env_iter
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| PathBuf::from(v).join("Library").join("Caches")),
        Os::Linux => env_iter
            .find(|(k, _)| k == "HOME")
            .map(|(_, v)| PathBuf::from(v).join(".cache")),
        Os::Windows => env_iter
            .find(|(k, _)| k == "APPDATA")
            .map(|(_, v)| PathBuf::from(v)),
    }
    .unwrap_or_else(|| {
        env::current_dir()
            .expect("should be able to get extension workdir")
            .join("caches")
    });

    // caches are unique per worktree-root-path
    let cache_key = worktree.root_path();

    let hex_digest = get_sha1_hex(&cache_key);
    let unique_dir_name = format!("jdtls-{}", hex_digest);
    Ok(base_cachedir.join(unique_dir_name))
}

fn get_sha1_hex(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn get_jdtls_launcher_from_path(worktree: &Worktree) -> Option<String> {
    let jdtls_executable_filename = match current_platform().0 {
        Os::Windows => "jdtls.bat",
        _ => "jdtls",
    };

    worktree.which(jdtls_executable_filename)
}

fn get_java_executable(configuration: &Option<Value>, worktree: &Worktree) -> zed::Result<PathBuf> {
    let java_executable_filename = match current_platform().0 {
        Os::Windows => "java.exe",
        _ => "java",
    };

    // Get executable from $JAVA_HOME
    if let Some(java_home) = get_java_home(configuration) {
        let java_executable = PathBuf::from(java_home)
            .join("bin")
            .join(java_executable_filename);
        if fs::metadata(&java_executable).is_ok_and(|stat| stat.is_file()) {
            return Ok(java_executable);
        }
    }
    // If we can't, try to get it from $PATH
    worktree
        .which(java_executable_filename)
        .map(PathBuf::from)
        .ok_or_else(|| "Could not find Java executable in JAVA_HOME or on PATH".to_string())
}

fn get_java_home(configuration: &Option<Value>) -> Option<String> {
    // try to read the value from settings
    if let Some(configuration) = configuration {
        if let Some(java_home) = configuration
            .pointer("/java/home")
            .and_then(|java_home_value| java_home_value.as_str())
        {
            return Some(java_home.to_string());
        }
    }

    // try to read the value from env (TODO I think we don't actually have access to the user env in here)
    match env::var("JAVA_HOME") {
        Ok(java_home) if !java_home.is_empty() => Some(java_home),
        _ => None,
    }
}

fn get_java_major_version(java_executable: &PathBuf) -> zed::Result<u32> {
    let program = java_executable
        .to_str()
        .ok_or_else(|| "Could not convert Java executable path to string".to_string())?;
    let output_bytes = Command::new(program).arg("-version").output()?.stderr;
    let output = String::from_utf8(output_bytes).map_err(|e| e.to_string())?;

    let major_version_regex =
        Regex::new(r#"version\s"(?P<major>\d+)(\.\d+\.\d+(_\d+)?)?"#).map_err(|e| e.to_string())?;
    let major_version = major_version_regex
        .captures_iter(&output)
        .find_map(|c| c.name("major").and_then(|m| m.as_str().parse::<u32>().ok()));

    if let Some(major_version) = major_version {
        Ok(major_version)
    } else {
        Err("Could not determine Java major version".to_string())
    }
}

fn find_equinox_launcher(jdtls_base_directory: &PathBuf) -> Result<PathBuf, String> {
    let plugins_dir = jdtls_base_directory.join("plugins");

    // if we have `org.eclipse.equinox.launcher.jar` use that
    let specific_launcher = plugins_dir.join("org.eclipse.equinox.launcher.jar");
    if specific_launcher.is_file() {
        return Ok(specific_launcher);
    }

    // else get the first file that matches the glob 'org.eclipse.equinox.launcher_*.jar'
    let entries = fs::read_dir(&plugins_dir)
        .map_err(|e| format!("Failed to read plugins directory: {}", e))?;

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map_or(false, |s| {
                        s.starts_with("org.eclipse.equinox.launcher_") && s.ends_with(".jar")
                    })
        })
        .ok_or_else(|| "Cannot find equinox launcher".to_string())
}

fn get_shared_config_path(jdtls_base_directory: &PathBuf) -> PathBuf {
    // TODO find out whether it makes sense to use config_linux_arm and config_mac_arm as well
    let config_to_use = match current_platform().0 {
        Os::Linux => "config_linux",
        Os::Mac => "config_mac",
        Os::Windows => "config_win",
    };
    jdtls_base_directory.join(config_to_use)
}

register_extension!(Java);
