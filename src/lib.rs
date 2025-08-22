mod debugger;
mod lsp;
use std::{
    collections::BTreeSet,
    env::current_dir,
    fs::{self, create_dir},
    path::{Path, PathBuf},
    str::FromStr,
};

use zed_extension_api::{
    self as zed, CodeLabel, CodeLabelSpan, DebugAdapterBinary, DebugTaskDefinition,
    DownloadedFileType, Extension, LanguageServerId, LanguageServerInstallationStatus, Os,
    StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest, Worktree,
    current_platform, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    lsp::{Completion, CompletionKind},
    make_file_executable, register_extension,
    serde_json::{self, Value, json},
    set_language_server_installation_status,
    settings::LspSettings,
};

use crate::{debugger::Debugger, lsp::LspWrapper};

const PROXY_FILE: &str = include_str!("proxy.mjs");
const DEBUG_ADAPTER_NAME: &str = "Java";
const PATH_TO_STR_ERROR: &str = "failed to convert path to string";

struct Java {
    cached_binary_path: Option<PathBuf>,
    cached_lombok_path: Option<PathBuf>,
    integrations: Option<(LspWrapper, Debugger)>,
}

impl Java {
    #[allow(dead_code)]
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

        if let Some(path_binary) = worktree.which(binary_name) {
            return Ok(PathBuf::from(path_binary));
        }

        // Check for latest version

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

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
        let prefix = PathBuf::from("jdtls");
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

        self.cached_binary_path = Some(binary_path.clone());

        Ok(binary_path)
    }

    fn lombok_jar_path(&mut self, language_server_id: &LanguageServerId) -> zed::Result<PathBuf> {
        // Use cached path if exists

        if let Some(path) = &self.cached_lombok_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        // Check for latest version

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
        let prefix = "lombok";
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

        self.cached_lombok_path = Some(jar_path.clone());

        Ok(jar_path)
    }
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
        let mut current_dir =
            current_dir().map_err(|err| format!("could not get current dir: {err}"))?;

        if current_platform().0 == Os::Windows {
            current_dir = current_dir
                .strip_prefix("/")
                .map_err(|err| err.to_string())?
                .to_path_buf();
        }

        let configuration =
            self.language_server_workspace_configuration(language_server_id, worktree)?;
        let java_home = configuration.as_ref().and_then(|configuration| {
            configuration
                .pointer("/java/home")
                .and_then(|java_home_value| {
                    java_home_value
                        .as_str()
                        .map(|java_home_str| java_home_str.to_string())
                })
        });

        let mut env = Vec::new();

        if let Some(java_home) = java_home {
            env.push(("JAVA_HOME".to_string(), java_home));
        }

        let mut args = vec![
            "--input-type=module".to_string(),
            "-e".to_string(),
            PROXY_FILE.to_string(),
            current_dir.to_str().ok_or(PATH_TO_STR_ERROR)?.to_string(),
            current_dir
                .join(self.language_server_binary_path(language_server_id, worktree)?)
                .to_str()
                .ok_or(PATH_TO_STR_ERROR)?
                .to_string(),
        ];

        // Add lombok as javaagent if settings.java.jdt.ls.lombokSupport.enabled is true
        let lombok_enabled = configuration
            .and_then(|configuration| {
                configuration
                    .pointer("/java/jdt/ls/lombokSupport/enabled")
                    .and_then(|enabled| enabled.as_bool())
            })
            .unwrap_or(false);

        if lombok_enabled {
            let lombok_jar_path = self.lombok_jar_path(language_server_id)?;
            let canonical_lombok_jar_path = current_dir
                .join(lombok_jar_path)
                .to_str()
                .ok_or(PATH_TO_STR_ERROR)?
                .to_string();

            args.push(format!("--jvm-arg=-javaagent:{canonical_lombok_jar_path}"));
        }

        // download debugger if not exists
        self.debugger()?.get_or_download(language_server_id)?;
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

        if self.integrations.is_some() {
            return Ok(Some(self.debugger()?.inject_plugin_into_options(options)?));
        }

        Ok(options)
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        // FIXME(Valentine Briese): I don't really like that we have a variable
        //                          here, there're probably some `Result` and/or
        //                          `Option` methods that would eliminate the
        //                          need for this, but at least this is easy to
        //                          read.

        let mut settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.settings);

        if !matches!(settings, Ok(Some(_))) {
            settings = self
                .language_server_initialization_options(language_server_id, worktree)
                .map(|initialization_options| {
                    initialization_options.and_then(|initialization_options| {
                        initialization_options.get("settings").cloned()
                    })
                })
        }

        settings
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

register_extension!(Java);
