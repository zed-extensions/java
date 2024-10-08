use zed_extension_api::{
    self as zed, lsp::CompletionKind, settings::LspSettings, CodeLabel, CodeLabelSpan,
};

struct Java;

impl zed::Extension for Java {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let java_home = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .settings
            .and_then(|settings| {
                settings.get("java_home").and_then(|java_home_value| {
                    java_home_value
                        .as_str()
                        .and_then(|java_home_str| Some(java_home_str.to_string()))
                })
            });
        let mut env = Vec::new();

        if let Some(java_home) = java_home {
            env.push(("JAVA_HOME".to_string(), java_home));
        }

        Ok(zed::Command {
            command: worktree
                .which("jdtls")
                .ok_or("could not find JDTLS in PATH")?,
            args: Vec::new(),
            env,
        })
    }

    fn label_for_completion(
        &self,
        _language_server_id: &zed::LanguageServerId,
        completion: zed::lsp::Completion,
    ) -> Option<zed::CodeLabel> {
        // uncomment when debugging completions
        // println!("Java completion: {completion:#?}");

        if let Some(kind) = completion.kind {
            match kind {
                CompletionKind::Field | CompletionKind::Constant => {
                    let modifiers = match kind {
                        CompletionKind::Field => "",
                        CompletionKind::Constant => "static final ",
                        _ => return None,
                    };
                    let property_type = completion.detail.as_ref().and_then(|detail| {
                        detail
                            .split_once(" : ")
                            .and_then(|(_, property_type)| Some(format!("{property_type} ")))
                    })?;
                    let semicolon = ";";
                    let code = format!("{modifiers}{property_type}{}{semicolon}", completion.label);

                    return Some(CodeLabel {
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
                    });
                }
                CompletionKind::Method => {
                    let (parameters, return_type) =
                        completion.detail.as_ref().and_then(|detail| {
                            let (left, return_type) = detail
                                .split_once(" : ")
                                .and_then(|(left, return_type)| {
                                    Some((left, format!("{return_type} ")))
                                })
                                .unwrap_or((detail, String::new()));

                            Some((&left[left.find('(')?..], return_type))
                        })?;
                    let name_and_parameters = format!("{}{parameters}", completion.label);
                    let braces = " {}";
                    let code = format!("{return_type}{name_and_parameters}{braces}");

                    return Some(CodeLabel {
                        spans: vec![
                            CodeLabelSpan::code_range(return_type.len()..code.len() - braces.len()),
                            CodeLabelSpan::literal(" : ", None),
                            CodeLabelSpan::code_range(0..return_type.len()),
                        ],
                        code,
                        filter_range: (0..completion.label.len()).into(),
                    });
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
                        Some(detail[..detail.len() - completion.label.len() - 1].to_string())
                    });
                    let mut spans = vec![CodeLabelSpan::code_range(
                        keyword.len()..code.len() - braces.len(),
                    )];

                    if let Some(namespace) = namespace {
                        spans.push(CodeLabelSpan::literal(format!(" ({namespace})"), None));
                    }

                    return Some(CodeLabel {
                        spans,
                        code,
                        filter_range: (0..completion.label.len()).into(),
                    });
                }
                CompletionKind::Snippet => {
                    return Some(CodeLabel {
                        code: String::new(),
                        spans: vec![CodeLabelSpan::literal(
                            format!("{} - {}", completion.label, completion.detail?),
                            None,
                        )],
                        filter_range: (0..completion.label.len()).into(),
                    });
                }
                CompletionKind::Keyword | CompletionKind::Variable => {
                    return Some(CodeLabel {
                        spans: vec![CodeLabelSpan::code_range(0..completion.label.len())],
                        filter_range: (0..completion.label.len()).into(),
                        code: completion.label,
                    });
                }
                CompletionKind::Constructor => {
                    let detail = completion.detail?;
                    let parameters = &detail[detail.find('(')?..];
                    let braces = " {}";
                    let code = format!("{}{parameters}{braces}", completion.label);

                    return Some(CodeLabel {
                        spans: vec![CodeLabelSpan::code_range(0..code.len() - braces.len())],
                        code,
                        filter_range: (0..completion.label.len()).into(),
                    });
                }
                _ => (),
            }
        }

        None
    }
}

zed::register_extension!(Java);
