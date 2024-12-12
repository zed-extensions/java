use std::fs;

use zed_extension_api::{
    self as zed, current_platform, download_file,
    http_client::{fetch, HttpMethod, HttpRequest},
    lsp::{Completion, CompletionKind},
    make_file_executable, register_extension,
    serde_json::{self, Value},
    set_language_server_installation_status,
    settings::LspSettings,
    CodeLabel, CodeLabelSpan, DownloadedFileType, Extension, LanguageServerId,
    LanguageServerInstallationStatus, Os, Worktree,
};

struct Java {
    cached_binary_path: Option<String>,
    cached_lombok_path: Option<String>,
}

impl Java {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<String> {
        // Use cached path if exists

        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).map_or(false, |stat| stat.is_file()) {
                return Ok(path.clone());
            }
        }

        // Use $PATH if binary is in it

        let (platform, _) = current_platform();
        let binary_name = match platform {
            Os::Windows => "jdtls.bat",
            _ => "jdtls",
        };

        if let Some(path_binary) = worktree.which(binary_name) {
            return Ok(path_binary);
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
                    .url("https://api.github.com/repos/eclipse-jdtls/eclipse.jdt.ls/tags")
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
        // Exclude ".tar.gz"
        let build_path = &latest_version_build[..latest_version_build.len() - 7];
        let binary_path = format!("{build_path}/bin/{binary_name}");

        // If latest version isn't installed,
        if !fs::metadata(&binary_path).map_or(false, |stat| stat.is_file()) {
            // then download it...

            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            download_file(&format!(
                "https://www.eclipse.org/downloads/download.php?file=/jdtls/milestones/{latest_version}/{latest_version_build}",
            ), build_path, DownloadedFileType::GzipTar)?;
            make_file_executable(&binary_path)?;

            // ...and delete other versions

            let entries = fs::read_dir(".")
                .map_err(|err| format!("failed to list working directory: {err}"))?;

            for entry in entries {
                let entry =
                    entry.map_err(|err| format!("failed to load directory entry: {err}"))?;

                dbg!(&entry);

                if entry.file_name().to_str() != Some(build_path) {
                    println!("should be removed");

                    if let Err(err) = fs::remove_dir_all(entry.path()) {
                        println!("failed to remove directory entry: {err}");
                    }
                } else {
                    println!("should not be removed");
                }
            }
        }

        // else use it

        self.cached_binary_path = Some(binary_path.clone());

        Ok(binary_path)
    }

    fn lombok_jar_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
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
            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            download_file(&lombok_url, &lombok_path, DownloadedFileType::Uncompressed)
                .map_err(|e| format!("failed to download file from {lombok_url} : {e}"))
                .inspect_err(|e| {
                    set_language_server_installation_status(
                        language_server_id,
                        &LanguageServerInstallationStatus::Failed(e.clone()),
                    );
                })?;
        }
        self.cached_lombok_path = Some(lombok_path.to_string());
        Ok(lombok_path.to_string())
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
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
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
        let java_home = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
            .initialization_options
            .and_then(|initialization_options| {
                initialization_options
                    .pointer("/settings/java/home")
                    .and_then(|java_home_value| {
                        java_home_value
                            .as_str()
                            .map(|java_home_str| java_home_str.to_string())
                    })
            });
        let mut env = Vec::new();

        if let Some(classpath) = classpath {
            env.push(("CLASSPATH".to_string(), classpath));
        }

        if let Some(java_home) = java_home {
            env.push(("JAVA_HOME".to_string(), java_home));
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
            command: self.language_server_binary_path(language_server_id, worktree)?,
            args,
            env,
        })
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
            _ => None,
        })
    }
}

register_extension!(Java);
