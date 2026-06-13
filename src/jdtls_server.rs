use std::env;

use zed_extension_api::{
    self as zed, CodeLabel, CodeLabelSpan, LanguageServerId, Worktree,
    lsp::{Completion, CompletionKind, Symbol, SymbolKind},
    serde_json::{Value, json},
    settings::LspSettings,
};

use crate::{
    component::Component,
    config::{get_java_home, get_jdtls_launcher, is_lombok_enabled},
    debugger::Debugger,
    jdk::Jdk,
    jdtls::{Jdtls, Lombok, build_jdtls_launch_args, get_jdtls_launcher_from_path},
    language_server::LanguageServer,
    proxy::Proxy,
    util::{path_to_file_uri, path_to_string},
};

pub struct JdtlsServer {
    pub jdtls: Jdtls,
    pub lombok: Lombok,
    pub proxy: Proxy,
    pub jdk: Jdk,
    pub debugger: Debugger,
    pub cached_workspace: Option<String>,
}

impl JdtlsServer {
    pub fn new() -> Self {
        Self {
            jdtls: Jdtls::new(),
            lombok: Lombok::new(),
            proxy: Proxy::new(),
            jdk: Jdk::new(),
            debugger: Debugger::new(),
            cached_workspace: None,
        }
    }
}

impl LanguageServer for JdtlsServer {
    const SERVER_ID: &'static str = "jdtls";

    fn command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        let current_dir =
            env::current_dir().map_err(|err| format!("Failed to get current directory: {err}"))?;

        let configuration = self.workspace_configuration(language_server_id, worktree)?;

        let mut env = Vec::new();

        if let Some(java_home) = get_java_home(&configuration, worktree) {
            env.push(("JAVA_HOME".to_string(), java_home));
        }

        let proxy_path = self
            .proxy
            .binary_path(&configuration, language_server_id, worktree)
            .map_err(|err| format!("Failed to get proxy binary path: {err}"))?;

        let mut args = vec![
            path_to_string(current_dir.clone())
                .map_err(|err| format!("Failed to convert current directory to string: {err}"))?,
        ];

        let lombok_jvm_arg = if is_lombok_enabled(&configuration) {
            let lombok_jar_path = self
                .lombok
                .get_or_download(language_server_id, &configuration, worktree)
                .map_err(|err| format!("Failed to get Lombok jar path: {err}"))?;
            let canonical_lombok_jar_path = path_to_string(current_dir.join(lombok_jar_path))
                .map_err(|err| format!("Failed to convert Lombok jar path to string: {err}"))?;

            Some(format!("-javaagent:{canonical_lombok_jar_path}"))
        } else {
            None
        };

        if let Some(launcher) = get_jdtls_launcher(&configuration, worktree) {
            args.push(launcher);
            if let Some(lombok_jvm_arg) = lombok_jvm_arg {
                args.push(format!("--jvm-arg={lombok_jvm_arg}"));
            }
        } else if let Some(launcher) = get_jdtls_launcher_from_path(worktree) {
            args.push(launcher);
            if let Some(lombok_jvm_arg) = lombok_jvm_arg {
                args.push(format!("--jvm-arg={lombok_jvm_arg}"));
            }
        } else {
            let jdtls_path = self
                .jdtls
                .get_or_download(language_server_id, &configuration, worktree)
                .map_err(|err| format!("Failed to get JDTLS binary path: {err}"))?;
            args.extend(
                build_jdtls_launch_args(
                    &jdtls_path,
                    &configuration,
                    worktree,
                    lombok_jvm_arg.into_iter().collect(),
                    language_server_id,
                    &mut self.jdk,
                )
                .map_err(|err| format!("Failed to build JDTLS launch arguments: {err}"))?,
            );
        }

        if let Err(err) =
            self.debugger
                .get_or_download(language_server_id, &configuration, worktree)
        {
            println!("Failed to download debugger: {err}");
        };

        self.cached_workspace = Some(worktree.root_path());

        Ok(zed::Command {
            command: proxy_path,
            args,
            env,
        })
    }

    fn initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        let mut options = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.initialization_options)
            .map_err(|err| format!("Failed to get LSP settings for worktree: {err}"))?
            .unwrap_or_else(|| json!({}));

        let options_obj = options
            .as_object_mut()
            .ok_or_else(|| "initialization_options is not a JSON object".to_string())?;
        if !options_obj.contains_key("workspaceFolders") {
            let uri = path_to_file_uri(&worktree.root_path());
            options_obj.insert("workspaceFolders".to_string(), json!([uri]));
        }

        let caps = options_obj
            .entry("extendedClientCapabilities")
            .or_insert_with(|| json!({}));
        let caps_obj = caps
            .as_object_mut()
            .ok_or_else(|| "extendedClientCapabilities is not a JSON object".to_string())?;
        caps_obj
            .entry("classFileContentsSupport")
            .or_insert(json!(true));
        caps_obj
            .entry("resolveAdditionalTextEditsSupport")
            .or_insert(json!(true));

        if self.debugger.plugin_path().is_some() {
            return Ok(Some(
                self.debugger
                    .inject_plugin_into_options(Some(options))
                    .map_err(|err| {
                        format!("Failed to inject debugger plugin into options: {err}")
                    })?,
            ));
        }

        Ok(Some(options))
    }

    fn workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        if let Ok(Some(settings)) = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .map(|lsp_settings| lsp_settings.settings)
        {
            Ok(Some(settings))
        } else {
            self.initialization_options(language_server_id, worktree)
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
            SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum => {
                let keyword = match symbol.kind {
                    SymbolKind::Class => "class ",
                    SymbolKind::Interface => "interface ",
                    SymbolKind::Enum => "enum ",
                    _ => return None,
                };
                let code = format!("{keyword}{name} {{}}");

                Some(CodeLabel {
                    spans: vec![CodeLabelSpan::code_range(0..keyword.len() + name.len())],
                    filter_range: (keyword.len()..keyword.len() + name.len()).into(),
                    code,
                })
            }
            SymbolKind::Method | SymbolKind::Function => {
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

                let mut spans = vec![
                    CodeLabelSpan::code_range(ret_start..ret_start + ret.len()),
                    CodeLabelSpan::literal(" ".to_string(), None),
                    CodeLabelSpan::code_range(name_start..name_start + method_name.len()),
                ];
                if !params.is_empty() {
                    spans.push(CodeLabelSpan::literal(params.to_string(), None));
                }

                let type_prefix_len = ret.len() + 1;
                let filter_end = type_prefix_len + method_name.len() + params.len();
                Some(CodeLabel {
                    spans,
                    filter_range: (type_prefix_len..filter_end).into(),
                    code,
                })
            }
            _ => None,
        }
    }
}
