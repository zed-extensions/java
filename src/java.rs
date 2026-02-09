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
    lsp::{Completion, CompletionKind, Symbol, SymbolKind},
    register_extension,
    serde_json::{Value, json},
    set_language_server_installation_status,
    settings::LspSettings,
};

use crate::{
    config::{get_java_home, get_jdtls_launcher, get_lombok_jar, is_lombok_enabled},
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
        configuration: &Option<Value>,
    ) -> zed::Result<PathBuf> {
        // Use cached path if exists

        if let Some(path) = &self.cached_binary_path
            && metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        // Check for latest version
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        match try_to_fetch_and_install_latest_jdtls(language_server_id, configuration) {
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
        configuration: &Option<Value>,
        worktree: &Worktree,
    ) -> zed::Result<PathBuf> {
        // Use user-configured path if provided
        if let Some(jar_path) = get_lombok_jar(configuration, worktree) {
            let path = PathBuf::from(&jar_path);
            self.cached_lombok_path = Some(path.clone());
            return Ok(path);
        }

        // Use cached path if exists
        if let Some(path) = &self.cached_lombok_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        match try_to_fetch_and_install_latest_lombok(language_server_id, configuration) {
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
            self.lsp()?
                .switch_workspace(worktree.root_path())
                .map_err(|err| {
                    format!("Failed to switch LSP workspace for debug adapter: {err}")
                })?;
        }

        Ok(DebugAdapterBinary {
            command: None,
            arguments: vec![],
            cwd: Some(worktree.root_path()),
            envs: vec![],
            request_args: StartDebuggingRequestArguments {
                request: self
                    .dap_request_kind(
                        adapter_name,
                        Value::from_str(config.config.as_str())
                            .map_err(|err| format!("Invalid JSON configuration: {err}"))?,
                    )
                    .map_err(|err| format!("Failed to determine debug request kind: {err}"))?,
                configuration: self
                    .debugger()?
                    .inject_config(worktree, config.config)
                    .map_err(|err| format!("Failed to inject debug configuration: {err}"))?,
            },
            connection: Some(zed::resolve_tcp_template(
                self.debugger()?
                    .start_session()
                    .map_err(|err| format!("Failed to start debug session: {err}"))?,
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
                    tcp_connection: Some(
                        self.debugger()?
                            .start_session()
                            .map_err(|err| format!("Failed to start debug session: {err}"))?,
                    ),
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
            env::current_dir().map_err(|err| format!("Failed to get current directory: {err}"))?;

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
            path_to_string(current_dir.clone())
                .map_err(|err| format!("Failed to convert current directory to string: {err}"))?,
        ];

        // Add lombok as javaagent if settings.java.jdt.ls.lombokSupport.enabled is true
        let lombok_jvm_arg = if is_lombok_enabled(&configuration) {
            let lombok_jar_path = self
                .lombok_jar_path(language_server_id, &configuration, worktree)
                .map_err(|err| format!("Failed to get Lombok jar path: {err}"))?;
            let canonical_lombok_jar_path = path_to_string(current_dir.join(lombok_jar_path))
                .map_err(|err| format!("Failed to convert Lombok jar path to string: {err}"))?;

            Some(format!("-javaagent:{canonical_lombok_jar_path}"))
        } else {
            None
        };

        self.init(worktree);

        // Check for user-configured JDTLS launcher first
        if let Some(launcher) = get_jdtls_launcher(&configuration, worktree) {
            args.push(launcher);
            if let Some(lombok_jvm_arg) = lombok_jvm_arg {
                args.push(format!("--jvm-arg={lombok_jvm_arg}"));
            }
        } else if let Some(launcher) = get_jdtls_launcher_from_path(worktree) {
            // if the user has `jdtls(.bat)` on their PATH, we use that
            args.push(launcher);
            if let Some(lombok_jvm_arg) = lombok_jvm_arg {
                args.push(format!("--jvm-arg={lombok_jvm_arg}"));
            }
        } else {
            // otherwise we launch ourselves
            args.extend(
                build_jdtls_launch_args(
                    &self
                        .language_server_binary_path(language_server_id, &configuration)
                        .map_err(|err| format!("Failed to get JDTLS binary path: {err}"))?,
                    &configuration,
                    worktree,
                    lombok_jvm_arg.into_iter().collect(),
                    language_server_id,
                )
                .map_err(|err| format!("Failed to build JDTLS launch arguments: {err}"))?,
            );
        }

        // download debugger if not exists
        if let Err(err) =
            self.debugger()?
                .get_or_download(language_server_id, &configuration, worktree)
        {
            println!("Failed to download debugger: {err}");
        };

        self.lsp()?
            .switch_workspace(worktree.root_path())
            .map_err(|err| format!("Failed to switch LSP workspace: {err}"))?;

        Ok(zed::Command {
            command: zed::node_binary_path()
                .map_err(|err| format!("Failed to get Node.js binary path: {err}"))?,
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
            self.lsp()?
                .switch_workspace(worktree.root_path())
                .map_err(|err| {
                    format!("Failed to switch LSP workspace for initialization: {err}")
                })?;
        }

        let options = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.initialization_options)
            .map_err(|err| format!("Failed to get LSP settings for worktree: {err}"))?;

        if self.debugger().is_ok_and(|v| v.loaded()) {
            return Ok(Some(
                self.debugger()?
                    .inject_plugin_into_options(options)
                    .map_err(|err| {
                        format!("Failed to inject debugger plugin into options: {err}")
                    })?,
            ));
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
                let namespace = completion.detail.and_then(|detail| {
                    if detail.len() > completion.label.len() {
                        let prefix_len = detail.len() - completion.label.len() - 1;
                        Some(detail[..prefix_len].to_string())
                    } else {
                        None
                    }
                });
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

    fn label_for_symbol(
        &self,
        _language_server_id: &LanguageServerId,
        symbol: Symbol,
    ) -> Option<CodeLabel> {
        let name = &symbol.name;

        match symbol.kind {
            SymbolKind::Class => {
                // code: "class Name {}" → Tree-sitter: class_declaration
                // display: "class Name"
                let keyword = "class ";
                let code = format!("{keyword}{name} {{}}");

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(0..keyword.len() + name.len())],
                    filter_range: (keyword.len()..keyword.len() + name.len()).into(),
                    code,
                })
            }
            SymbolKind::Interface => {
                let keyword = "interface ";
                let code = format!("{keyword}{name} {{}}");

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(0..keyword.len() + name.len())],
                    filter_range: (keyword.len()..keyword.len() + name.len()).into(),
                    code,
                })
            }
            SymbolKind::Enum => {
                let keyword = "enum ";
                let code = format!("{keyword}{name} {{}}");

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(0..keyword.len() + name.len())],
                    filter_range: (keyword.len()..keyword.len() + name.len()).into(),
                    code,
                })
            }
            SymbolKind::Constructor => {
                // jdtls: "ClassName(Type, Type)"
                let ctor_name = name.split('(').next().unwrap_or(name);
                let rest = &name[ctor_name.len()..];
                // Wrap in matching class for constructor_declaration AST node
                let prefix = format!("class {ctor_name} {{ ");
                let code = format!("{prefix}{ctor_name}() {{}} }}");
                let ctor_start = prefix.len();

                let mut spans = vec![
                    CodeLabelSpan::code_range(ctor_start..ctor_start + ctor_name.len()),
                ];
                if !rest.is_empty() {
                    spans.push(CodeLabelSpan::literal(rest.to_string(), None));
                }

                Some(CodeLabel {
                    spans,
                    filter_range: (0..name.len()).into(),
                    code,
                })
            }
            SymbolKind::Method | SymbolKind::Function => {
                // jdtls: "methodName(Type, Type) : ReturnType" or "methodName(Type)"
                // display: "ReturnType methodName(Type, Type)" (Java declaration order)
                let method_name = name.split('(').next().unwrap_or(name);
                let after_name = &name[method_name.len()..];

                let (params, return_type) = if let Some((p, r)) = after_name.split_once(" : ") {
                    (p, Some(r))
                } else {
                    (after_name, None)
                };

                let ret = return_type.unwrap_or("void");
                let class_open = "class _ { ";
                let code = format!("{class_open}{ret} {method_name}() {{}} }}");

                let ret_start = class_open.len();
                let name_start = ret_start + ret.len() + 1;

                // Display: "void methodName(String, int)"
                let mut spans = vec![
                    CodeLabelSpan::code_range(ret_start..ret_start + ret.len()),
                    CodeLabelSpan::literal(" ".to_string(), None),
                    CodeLabelSpan::code_range(name_start..name_start + method_name.len()),
                ];
                if !params.is_empty() {
                    spans.push(CodeLabelSpan::literal(params.to_string(), None));
                }

                // filter on "methodName(params)" portion of displayed text
                let type_prefix_len = ret.len() + 1; // "void "
                let filter_end = type_prefix_len + method_name.len() + params.len();
                Some(CodeLabel {
                    spans,
                    filter_range: (type_prefix_len..filter_end).into(),
                    code,
                })
            }
            SymbolKind::Field | SymbolKind::Property => {
                // jdtls: "fieldName : Type" or just "fieldName"
                // display: "Type fieldName" (Java declaration order)
                if let Some((field_name, field_type)) = name.split_once(" : ") {
                    let class_open = "class _ { ";
                    let code = format!("{class_open}{field_type} {field_name}; }}");

                    let type_start = class_open.len();
                    let name_start = type_start + field_type.len() + 1;

                    // Display: "String fieldName"
                    let spans = vec![
                        CodeLabelSpan::code_range(type_start..type_start + field_type.len()),
                        CodeLabelSpan::literal(" ".to_string(), None),
                        CodeLabelSpan::code_range(name_start..name_start + field_name.len()),
                    ];

                    let type_prefix_len = field_type.len() + 1; // "String "
                    Some(CodeLabel {
                        spans,
                        filter_range: (type_prefix_len..type_prefix_len + field_name.len()).into(),
                        code,
                    })
                } else {
                    // No type info, just show the name
                    let class_open = "class _ { int ";
                    let code = format!("{class_open}{name}; }}");
                    let name_start = class_open.len();

                    Some(CodeLabel {
                        spans: vec![CodeLabelSpan::code_range(
                            name_start..name_start + name.len(),
                        )],
                        filter_range: (0..name.len()).into(),
                        code,
                    })
                }
            }
            SymbolKind::Constant => {
                // Wrap in class; ALL_CAPS names get @constant from highlights.scm regex
                let class_open = "class _ { static final int ";
                let code = format!("{class_open}{name}; }}");
                let name_start = class_open.len();

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(
                        name_start..name_start + name.len(),
                    )],
                    filter_range: (0..name.len()).into(),
                    code,
                })
            }
            SymbolKind::EnumMember => {
                // Wrap in enum for enum_constant AST node → @constant highlight
                let prefix = "enum _ { ";
                let code = format!("{prefix}{name} }}");
                let name_start = prefix.len();

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(
                        name_start..name_start + name.len(),
                    )],
                    filter_range: (0..name.len()).into(),
                    code,
                })
            }
            SymbolKind::Variable => {
                let class_open = "class _ { int ";
                let code = format!("{class_open}{name}; }}");
                let name_start = class_open.len();

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(
                        name_start..name_start + name.len(),
                    )],
                    filter_range: (0..name.len()).into(),
                    code,
                })
            }
            SymbolKind::Package | SymbolKind::Module | SymbolKind::Namespace => {
                let keyword = "package ";
                let code = format!("{keyword}{name};");

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(0..keyword.len() + name.len())],
                    filter_range: (keyword.len()..keyword.len() + name.len()).into(),
                    code,
                })
            }
            _ => None,
        }
    }
}

register_extension!(Java);
