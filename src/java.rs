mod config;
mod debugger;
mod downloadable;
mod gradle_bridge;
mod gradle_ls;
mod gradle_ls_server;
mod jdk;
mod jdtls;
mod jdtls_server;
mod language_server;
mod lsp;
mod proxy;
mod task;
mod util;

use std::str::FromStr;

use zed_extension_api::{
    self as zed, CodeLabel, DebugAdapterBinary, DebugTaskDefinition, Extension, LanguageServerId,
    StartDebuggingRequestArguments, StartDebuggingRequestArgumentsRequest, Worktree,
    lsp::{Completion, Symbol},
    register_extension,
    serde_json::{Value, json},
};

use crate::{
    downloadable::Downloadable, gradle_ls_server::GradleLsServer, jdtls_server::JdtlsServer,
    language_server::LanguageServer,
};

const DEBUG_ADAPTER_NAME: &str = "Java";

struct Java {
    jdtls_server: JdtlsServer,
    gradle_ls_server: GradleLsServer,
}

impl Extension for Java {
    fn new() -> Self
    where
        Self: Sized,
    {
        Self {
            jdtls_server: JdtlsServer::new(),
            gradle_ls_server: GradleLsServer::new(),
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self.jdtls_server.command(language_server_id, worktree),
            GradleLsServer::SERVER_ID => {
                self.gradle_ls_server.command(language_server_id, worktree)
            }
            id => Err(format!("Unknown language server: {id}")),
        }
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .initialization_options(language_server_id, worktree),
            GradleLsServer::SERVER_ID => self
                .gradle_ls_server
                .initialization_options(language_server_id, worktree),
            _ => Ok(None),
        }
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<Value>> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .workspace_configuration(language_server_id, worktree),
            GradleLsServer::SERVER_ID => self
                .gradle_ls_server
                .workspace_configuration(language_server_id, worktree),
            _ => Ok(None),
        }
    }

    fn label_for_completion(
        &self,
        language_server_id: &LanguageServerId,
        completion: Completion,
    ) -> Option<CodeLabel> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .label_for_completion(language_server_id, completion),
            _ => None,
        }
    }

    fn label_for_symbol(
        &self,
        language_server_id: &LanguageServerId,
        symbol: Symbol,
    ) -> Option<CodeLabel> {
        match language_server_id.as_ref() {
            JdtlsServer::SERVER_ID => self
                .jdtls_server
                .label_for_symbol(language_server_id, symbol),
            _ => None,
        }
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        config: DebugTaskDefinition,
        _user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> zed_extension_api::Result<DebugAdapterBinary, String> {
        if !self.jdtls_server.debugger.loaded() {
            return Err("Debugger plugin is not loaded".to_string());
        }

        if adapter_name != DEBUG_ADAPTER_NAME {
            return Err(format!(
                "Cannot create binary for adapter \"{adapter_name}\""
            ));
        }

        let workspace = worktree.root_path();

        Ok(DebugAdapterBinary {
            command: None,
            arguments: vec![],
            cwd: Some(workspace.clone()),
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
                    .jdtls_server
                    .debugger
                    .inject_config(worktree, config.config)
                    .map_err(|err| format!("Failed to inject debug configuration: {err}"))?,
            },
            connection: Some(zed::resolve_tcp_template(
                self.jdtls_server
                    .debugger
                    .start_session(&workspace)
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
        if !self.jdtls_server.debugger.loaded() {
            return Err("Debugger plugin is not loaded".to_string());
        }

        let workspace = self
            .jdtls_server
            .cached_workspace
            .as_deref()
            .ok_or("LSP workspace not initialized yet")?;

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
                        self.jdtls_server
                            .debugger
                            .start_session(workspace)
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
}

register_extension!(Java);
