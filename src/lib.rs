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
        if let Some(kind) = completion.kind {
            match kind {
                CompletionKind::Field => {
                    let (_, field_type) = completion.detail.as_ref()?.split_once(" : ")?;
                    let code = format!("{field_type} {};", completion.label);

                    return Some(CodeLabel {
                        spans: vec![
                            CodeLabelSpan::code_range(field_type.len() + 1..code.len() - 1),
                            CodeLabelSpan::literal(" : ", None),
                            CodeLabelSpan::code_range(0..field_type.len()),
                        ],
                        code,
                        filter_range: (0..completion.label.len()).into(),
                    });
                }
                CompletionKind::Method => {
                    let (left, return_type) = completion.detail.as_ref()?.split_once(" : ")?;
                    let parameters = &left[left.find('(')?..];
                    let name_and_parameters = format!("{}{parameters}", completion.label);
                    let braces = " {}";
                    let code = format!("{return_type} {name_and_parameters}{braces}");

                    return Some(CodeLabel {
                        spans: vec![
                            CodeLabelSpan::code_range(
                                return_type.len() + 1..code.len() - braces.len(),
                            ),
                            CodeLabelSpan::literal(" : ", None),
                            CodeLabelSpan::code_range(0..return_type.len()),
                        ],
                        code,
                        filter_range: (0..completion.label.len()).into(),
                    });
                }
                CompletionKind::Class => {
                    let class = "class ";
                    let braces = " {}";
                    let code = format!("{class}{}{braces}", completion.label);
                    let detail = completion.detail?;
                    let namespace = &detail[..detail.len() - completion.label.len() - 1];

                    return Some(CodeLabel {
                        spans: vec![
                            CodeLabelSpan::code_range(class.len()..code.len() - braces.len()),
                            CodeLabelSpan::literal(format!(" ({namespace})"), None),
                        ],
                        code,
                        filter_range: (0..completion.label.len()).into(),
                    });
                }
                _ => (),
            }
        }

        println!("unhandled Java completion: {completion:#?}"); // warn

        None
    }
}

zed::register_extension!(Java);
