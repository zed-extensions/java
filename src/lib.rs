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
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        Ok(zed::Command {
            command: worktree
                .which("jdtls")
                .ok_or("could not find JDTLS in PATH")?,
            args: vec![],
            env: vec![],
        })
    }
}

zed::register_extension!(Java);
