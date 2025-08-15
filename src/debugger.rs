use std::{env::current_dir, fs, net::Ipv4Addr, path::PathBuf};

use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Os,
    TcpArguments, Worktree, current_platform, download_file,
    http_client::{HttpMethod, HttpRequest, fetch},
    serde_json::{self, Value, json},
    set_language_server_installation_status,
};

use crate::lsp::LspClient;

const MAVEN_SEARCH_URL: &str =
    "https://search.maven.org/solrsearch/select?q=a:com.microsoft.java.debug.plugin";

pub struct Debugger {
    path: Option<PathBuf>,
}

impl Debugger {
    pub fn new() -> Debugger {
        Debugger { path: None }
    }

    pub fn get_or_download(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> zed::Result<PathBuf> {
        let prefix = "debugger";

        if let Some(path) = &self.path {
            if fs::metadata(path).is_ok_and(|stat| stat.is_file()) {
                return Ok(path.clone());
            }
        }

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let res = fetch(
            &HttpRequest::builder()
                .method(HttpMethod::Get)
                .url(MAVEN_SEARCH_URL)
                .build()?,
        );

        // Maven loves to be down, trying to resolve it gracefully
        if let Err(err) = &res {
            if !fs::metadata(prefix).is_ok_and(|stat| stat.is_dir()) {
                return Err(err.to_owned());
            }

            // If it's not a 5xx code, then return an error.
            if !err.contains("status code 5") {
                return Err(err.to_owned());
            }

            let exists = fs::read_dir(&prefix)
                .ok()
                .map(|dir| dir.last().map(|v| v.ok()))
                .flatten()
                .flatten();

            if let Some(file) = exists {
                if !file.metadata().is_ok_and(|stat| stat.is_file()) {
                    return Err(err.to_owned());
                }

                if !file
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".jar"))
                {
                    return Err(err.to_owned());
                }

                let jar_path = PathBuf::from(prefix).join(file.file_name());
                self.path = Some(jar_path.clone());

                return Ok(jar_path);
            }
        }

        let maven_response_body = serde_json::from_slice::<Value>(&res?.body)
            .map_err(|err| format!("failed to deserialize Maven response: {err}"))?;

        let latest_version = maven_response_body
            .pointer("/response/docs/0/latestVersion")
            .map(|v| v.as_str())
            .flatten()
            .ok_or("Malformed maven response")?;

        let artifact = maven_response_body
            .pointer("/response/docs/0/a")
            .map(|v| v.as_str())
            .flatten()
            .ok_or("Malformed maven response")?;

        let jar_name = format!("{artifact}-{latest_version}.jar");
        let jar_path = PathBuf::from(prefix).join(&jar_name);

        if !fs::metadata(&jar_path).is_ok_and(|stat| stat.is_file()) {
            if let Err(err) = fs::remove_dir_all(prefix) {
                println!("failed to remove directory entry: {err}");
            }

            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            fs::create_dir(prefix).map_err(|err| err.to_string())?;

            let url = format!(
                "https://repo1.maven.org/maven2/com/microsoft/java/{artifact}/{latest_version}/{jar_name}"
            );

            download_file(
                url.as_str(),
                jar_path
                    .to_str()
                    .ok_or("Failed to convert path to string")?,
                DownloadedFileType::Uncompressed,
            )
            .map_err(|err| format!("Failed to download {url} {err}"))?;
        }

        self.path = Some(jar_path.clone());
        Ok(jar_path)
    }

    pub fn start_session(&self, worktree: &Worktree) -> zed::Result<TcpArguments> {
        let port = LspClient::request(
            worktree,
            "workspace/executeCommand",
            json!({
                "command": "vscode.java.startDebugSession"
            }),
        )?
        .get("result")
        .map(|v| v.as_u64())
        .flatten()
        .ok_or("Failed to read lsp proxy debug response")?;

        Ok(TcpArguments {
            host: Ipv4Addr::LOCALHOST.to_bits(),
            port: port as u16,
            timeout: Some(60_000),
        })
    }

    pub fn inject_plugin_into_options(
        &self,
        initialization_options: Option<Value>,
    ) -> zed::Result<Value> {
        let mut current_dir =
            current_dir().map_err(|err| format!("could not get current dir: {err}"))?;

        if current_platform().0 == Os::Windows {
            current_dir = current_dir
                .strip_prefix("/")
                .map_err(|err| err.to_string())?
                .to_path_buf();
        }

        let canonical_path = Value::String(
            current_dir
                .join(self.path.as_ref().ok_or("Debugger is not loaded yet")?)
                .to_string_lossy()
                .to_string(),
        );

        match initialization_options {
            None => {
                return Ok(json!({
                    "bundles": [canonical_path]
                }));
            }
            Some(options) => {
                let mut options = options.clone();

                // ensure bundles field exists
                let mut bundles = options
                    .get_mut("bundles")
                    .unwrap_or(&mut Value::Array(vec![]))
                    .take();

                let bundles_vec = bundles
                    .as_array_mut()
                    .ok_or("Invalid initialization_options format")?;

                if !bundles_vec.contains(&canonical_path) {
                    bundles_vec.push(canonical_path);
                }

                options["bundles"] = bundles;

                return Ok(options);
            }
        }
    }
}
