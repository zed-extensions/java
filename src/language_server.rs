use zed_extension_api::{
    self as zed, CodeLabel, LanguageServerId, Worktree,
    lsp::{Completion, Symbol},
    serde_json::Value,
    settings::LspSettings,
};

pub trait LanguageServer {
    const SERVER_ID: &'static str;

    fn command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command>;

    fn initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>>;

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
                .map(|opts| opts.and_then(|o| o.get("settings").cloned()))
        }
    }

    fn label_for_completion(
        &self,
        _language_server_id: &LanguageServerId,
        _completion: Completion,
    ) -> Option<CodeLabel> {
        None
    }

    fn label_for_symbol(
        &self,
        _language_server_id: &LanguageServerId,
        _symbol: Symbol,
    ) -> Option<CodeLabel> {
        None
    }
}
