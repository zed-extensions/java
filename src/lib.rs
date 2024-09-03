use zed_extension_api as zed;

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
        let java_home =
            zed::settings::LspSettings::for_worktree(language_server_id.as_ref(), worktree)?
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
}

zed::register_extension!(Java);
