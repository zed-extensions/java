use zed_extension_api::{
    self as zed, lsp::CompletionKind, settings::LspSettings, CodeLabel, CodeLabelSpan,
};

struct Java {
    cached_binary_path: Option<String>,
    cached_lombok_path: Option<String>,
}

impl Java {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<String> {
        // Quickly return if the binary path is already cached
        // Expect binary path to be validated when setting the cache so no checking is done here
        if let Some(path) = &self.cached_binary_path {
            return Ok(path.clone());
        }

        // Determine the binary name based on the current platform
        let (platform, _) = zed::current_platform();
        let binary_name = match platform {
            zed::Os::Windows => "jdtls.bat",
            _ => "jdtls",
        }
        .to_string();

        // Use binary available on PATH if it exists
        if let Some(path) = worktree.which(&binary_name) {
            // Probably we want to check if the binary is executable too here
            if std::fs::metadata(&path).map_or(false, |stat| stat.is_file()) {
                self.cached_binary_path = Some(path.clone());
                return Ok(path.clone());
            }
        }

        // Attempt to install locally

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        // Use version specified in settings or default version
        let version = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .settings
            .and_then(|settings| {
                settings.get("jdtls_version").and_then(|version| {
                    version
                        .as_str()
                        .map(|version_str| version_str.trim().to_string())
                })
            })
            // Probably we can get the latest version from Maven?
            .unwrap_or("1.40.0".to_string());

        // Prebuilt milestone versions available at:
        // https://download.eclipse.org/jdtls/milestones/{version}
        // Tarball filename is specified at
        // https://download.eclipse.org/jdtls/milestones/{version}/latest.txt

        let install_prefix = format!("jdt-language-server-{version}");
        let binary_path = std::path::Path::new(&install_prefix)
            .join("bin")
            .join(binary_name)
            .to_string_lossy()
            .to_string();

        // Validate binary
        if !std::fs::metadata(&binary_path).map_or(false, |stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            // Download latest.txt to get the tarball filename
            let latest_txt_path = format!("{install_prefix}-latest.txt");
            let latest_txt_url =
                format!("https://download.eclipse.org/jdtls/milestones/{version}/latest.txt");
            zed::download_file(
                &latest_txt_url,
                &latest_txt_path,
                zed::DownloadedFileType::Uncompressed,
            )
            .map_err(|e| format!("failed to download file: {e}"))
            .inspect_err(|e| {
                zed::set_language_server_installation_status(
                    language_server_id,
                    &zed::LanguageServerInstallationStatus::Failed(e.clone()),
                );
            })?;
            let tarball_name = std::fs::read_to_string(&latest_txt_path)
                .map_err(|e| format!("failed to read file {latest_txt_path} : {e}"))
                .inspect_err(|e| {
                    zed::set_language_server_installation_status(
                        language_server_id,
                        &zed::LanguageServerInstallationStatus::Failed(e.clone()),
                    );
                })?
                .trim()
                .to_string();

            // Download tarball and extract
            let tarball_url =
                format!("https://download.eclipse.org/jdtls/milestones/{version}/{tarball_name}");

            zed::download_file(
                &tarball_url,
                &install_prefix,
                zed::DownloadedFileType::GzipTar,
            )
            .map_err(|e| format!("failed to download file from {tarball_url} : {e}"))
            .inspect_err(|e| {
                zed::set_language_server_installation_status(
                    language_server_id,
                    &zed::LanguageServerInstallationStatus::Failed(e.clone()),
                );
            })?;

            zed::make_file_executable(&binary_path)
                .map_err(|e| format!("failed to make file {binary_path} executable: {e}"))
                .inspect_err(|e| {
                    zed::set_language_server_installation_status(
                        language_server_id,
                        &zed::LanguageServerInstallationStatus::Failed(e.clone()),
                    );
                })?;
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path.clone())
    }

    fn lombok_jar_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<String> {
        // Quickly return if the lombok path is already cached
        // Expect lombok path to be validated when setting the cache so no checking is done here
        if let Some(path) = &self.cached_lombok_path {
            return Ok(path.clone());
        }

        // Use lombok version specified in settings
        // Unspecified version (None here) defaults to the latest version
        let lombok_version = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .settings
            .and_then(|settings| {
                settings
                    .get("lombok_version")
                    .and_then(|version| version.as_str())
                    .map(|version_str| version_str.to_string())
            })
            .map(|version| version.trim().to_string());

        // Download lombok jar
        // https://projectlombok.org/downloads/lombok.jar always points to the latest version
        // https://projectlombok.org/downloads/lombok-{version}.jar points to the specified version
        let (lombok_url, lombok_path) = match lombok_version {
            Some(v) => (
                format!("https://projectlombok.org/downloads/lombok-{v}.jar"),
                format!("lombok-{v}.jar"),
            ),
            None => (
                "https://projectlombok.org/downloads/lombok.jar".to_string(),
                "lombok.jar".to_string(),
            ),
        };
        // Do not download if lombok jar already exists
        if !std::fs::metadata(&lombok_path).map_or(false, |stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            zed::download_file(
                &lombok_url,
                &lombok_path,
                zed::DownloadedFileType::Uncompressed,
            )
            .map_err(|e| format!("failed to download file from {lombok_url} : {e}"))
            .inspect_err(|e| {
                zed::set_language_server_installation_status(
                    language_server_id,
                    &zed::LanguageServerInstallationStatus::Failed(e.clone()),
                );
            })?;
        }
        self.cached_lombok_path = Some(lombok_path.to_string());
        Ok(lombok_path.to_string())
    }
}

impl zed::Extension for Java {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self {
            cached_binary_path: None,
            cached_lombok_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let classpath = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .settings
            .and_then(|settings| {
                settings.get("classpath").and_then(|classpath_value| {
                    classpath_value
                        .as_str()
                        .map(|classpath_str| classpath_str.to_string())
                })
            });
        let mut env = Vec::new();

        if let Some(classpath) = classpath {
            env.push(("CLASSPATH".to_string(), classpath));
        }

        let mut args = Vec::new();

        // Add lombok as javaagent if initialization_options.settings.java.jdt.ls.lombokSupport.enabled is true
        let lombok_enabled = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .initialization_options
            .and_then(|initialization_options| {
                initialization_options
                    .pointer("settings/java/jdt/ls/lombokSupport/enabled")
                    .and_then(|enabled| enabled.as_bool())
            })
            .unwrap_or(false);
        if lombok_enabled {
            let lombok_jar_path = self.lombok_jar_path(language_server_id, worktree)?;
            let lombok_jar_full_path = std::env::current_dir()
                .map_err(|e| format!("could not get current dir: {e}"))?
                .join(&lombok_jar_path)
                .to_string_lossy()
                .to_string();
            args.push(format!("--jvm-arg=-javaagent:{lombok_jar_full_path}"));
        }

        Ok(zed::Command {
            command: self
                .language_server_binary_path(language_server_id, worktree)
                .map_err(|e| format!("could not find language server binary: {e}"))?,
            args,
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
                    let detail = completion.detail?;
                    let (left, return_type) = detail
                        .split_once(" : ")
                        .and_then(|(left, return_type)| Some((left, format!("{return_type} "))))
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

                    return Some(CodeLabel {
                        spans,
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
