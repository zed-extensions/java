mod config;
mod debugger;
mod jdk;
mod jdtls;
mod lsp;
mod util;

use std::{
    env,
    fs::{self, metadata},
    path::PathBuf,
    str::FromStr,
};

use zed_extension_api::{
    self as zed, CodeLabel, CodeLabelSpan, DebugAdapterBinary, DebugTaskDefinition, Extension,
    LanguageServerId, LanguageServerInstallationStatus, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, Worktree,
    lsp::{Completion, CompletionKind},
    register_extension,
    serde_json::{Value, json},
    set_language_server_installation_status,
    settings::LspSettings,
};

use crate::{
    config::{get_java_home, is_lombok_enabled},
    debugger::Debugger,
    jdtls::{
        build_jdtls_launch_args, find_latest_local_jdtls, find_latest_local_lombok,
        get_jdtls_launcher_from_path, try_to_fetch_and_install_latest_jdtls,
        try_to_fetch_and_install_latest_lombok,
    },
    lsp::LspWrapper,
    util::path_to_string,
};

const PROXY_FILE: &str = include_str!("proxy.mjs");
const DEBUG_ADAPTER_NAME: &str = "Java";
const LSP_INIT_ERROR: &str = "Lsp client is not initialized yet";

struct Java {
    cached_binary_path: Option<PathBuf>,
    cached_lombok_path: Option<PathBuf>,
    integrations: Option<(LspWrapper, Debugger)>,
}

impl Java {
    fn lsp(&mut self) -> zed::Result<&LspWrapper> {
        self.integrations
            .as_ref()
            .ok_or(LSP_INIT_ERROR.to_string())
            .map(|v| &v.0)
    }

    fn debugger(&mut self) -> zed::Result<&mut Debugger> {
        self.integrations
            .as_mut()
            .ok_or(LSP_INIT_ERROR.to_string())
            .map(|v| &mut v.1)
    }

    fn init(&mut self, worktree: &Worktree) {
        // Initialize lsp client and debugger

        if self.integrations.is_none() {
            let lsp = LspWrapper::new(worktree.root_path());
            let debugger = Debugger::new(lsp.clone());

            self.integrations = Some((lsp, debugger));
        }
    }

    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        // Use cached path if exists

        if let Some(path) = &self.cached_binary_path
            && metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        let configuration =
            self.language_server_workspace_configuration(language_server_id, worktree)?;

        // Check for latest version
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        match try_to_fetch_and_install_latest_jdtls(language_server_id, &configuration) {
            Ok(path) => {
                self.cached_binary_path = Some(path.clone());
                Ok(path)
            }
            Err(e) => {
                if let Some(local_version) = find_latest_local_jdtls() {
                    self.cached_binary_path = Some(local_version.clone());
                    Ok(local_version)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn lombok_jar_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        if let Some(path) = &self.cached_lombok_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        let configuration =
            self.language_server_workspace_configuration(language_server_id, worktree)?;

        match try_to_fetch_and_install_latest_lombok(language_server_id, &configuration) {
            Ok(path) => {
                self.cached_lombok_path = Some(path.clone());
                Ok(path)
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

        if let Some(java_home) = get_java_home(&configuration, worktree) {
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
        let lombok_jvm_arg = if is_lombok_enabled(&configuration) {
            let lombok_jar_path = self.lombok_jar_path(language_server_id, worktree)?;
            let canonical_lombok_jar_path = path_to_string(current_dir.join(lombok_jar_path))?;

            Some(format!("-javaagent:{canonical_lombok_jar_path}"))
        } else {
            None
        };

        self.init(worktree);

        if let Some(launcher) = get_jdtls_launcher_from_path(worktree) {
            // if the user has `jdtls(.bat)` on their PATH, we use that
            args.push(launcher);
            if let Some(lombok_jvm_arg) = lombok_jvm_arg {
                args.push(format!("--jvm-arg={lombok_jvm_arg}"));
            }
        } else {
            // otherwise we launch ourselves
            args.extend(build_jdtls_launch_args(
                &self.language_server_binary_path(language_server_id, worktree)?,
                &configuration,
                worktree,
                lombok_jvm_arg.into_iter().collect(),
                language_server_id,
            )?);
        }

        // download debugger if not exists
        if let Err(err) = self
            .debugger()?
            .get_or_download(language_server_id, &configuration)
        {
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

register_extension!(Java);
