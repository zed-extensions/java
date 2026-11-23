mod completions;
mod decompile;
mod http;
mod log;
mod output;
mod pending;

use completions::{process_completions, sanitize_resolved_completion};
use decompile::{DecompileCoordinator, Priority, RewriteJob, RewriteMode, SharedWriter};
use http::handle_http;
use output::Output;
use pending::PendingResponses;
use proxy_common::{
    contains_subslice, encode_lsp, file_uri_to_path, parse_lsp_content, raw_has_id,
    spawn_parent_monitor, LspReader,
};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::{self, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RewriteKind {
    Locations,
    Documentation,
    Completion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackedRequest {
    token: u64,
    method: String,
    rewrite: RewriteKind,
    original_params: Option<Value>,
}

impl TrackedRequest {
    fn new(token: u64, method: &str, rewrite: RewriteKind, original_params: Option<Value>) -> Self {
        Self {
            token,
            method: method.to_string(),
            rewrite,
            original_params,
        }
    }
}

#[derive(Default)]
struct SuppressedResponses {
    ids: HashSet<Value>,
}

impl SuppressedResponses {
    fn insert(&mut self, id: Value) {
        self.ids.insert(id);
    }

    fn contains(&self, id: &Value) -> bool {
        self.ids.contains(id)
    }

    fn take(&mut self, id: &Value) -> bool {
        self.ids.remove(id)
    }
}

type TrackedRequests = Arc<Mutex<HashMap<Value, TrackedRequest>>>;
type ActiveRewrites = Arc<Mutex<HashMap<Value, u64>>>;
type SharedSuppressedResponses = Arc<Mutex<SuppressedResponses>>;
type LatestWorkspaceJob = Arc<Mutex<Option<u64>>>;

struct StdinContext {
    writer: SharedWriter,
    alive: Arc<AtomicBool>,
    tracked: TrackedRequests,
    active: ActiveRewrites,
    jobs: Arc<AtomicU64>,
    decompile: DecompileCoordinator,
    output: Output,
    suppressed: SharedSuppressedResponses,
    latest_workspace: LatestWorkspaceJob,
}

struct StdoutContext {
    pending: Arc<PendingResponses>,
    alive: Arc<AtomicBool>,
    tracked: TrackedRequests,
    active: ActiveRewrites,
    decompile: DecompileCoordinator,
    output: Output,
    suppressed: SharedSuppressedResponses,
    latest_workspace: LatestWorkspaceJob,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputRoute {
    Forward,
    Consumed,
}

#[derive(Debug, PartialEq)]
enum OutputRoute {
    Raw(Vec<u8>),
    Value(Value),
    Consumed,
}

fn main() {
    let output = Output::start();
    log::init(output.clone());

    let args: Vec<String> = env::args().skip(1).collect();

    if args.len() < 2 {
        eprintln!("Usage: java-lsp-proxy <workdir> <bin> [args...]");
        lsp_error!("Usage: java-lsp-proxy <workdir> <bin> [args...]");
        process::exit(1);
    }

    let workdir = &args[0];
    let bin = &args[1];
    let child_args = &args[2..];

    lsp_info!("java-lsp-proxy starting: bin={bin}, workdir={workdir}");

    let proxy_id = hex_encode(
        env::current_dir()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches('/'),
    );

    // Spawn JDTLS (use shell on Windows for .bat files)
    let mut cmd = Command::new(bin);
    cmd.args(child_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    #[cfg(windows)]
    if bin.ends_with(".bat") || bin.ends_with(".cmd") {
        cmd = Command::new("cmd");
        cmd.arg("/C")
            .arg(bin)
            .args(child_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
    }

    let mut child = cmd.spawn().unwrap_or_else(|e| {
        eprintln!("Failed to spawn {bin}: {e}");
        lsp_error!("Failed to spawn {bin}: {e}");
        process::exit(1);
    });

    lsp_info!("JDTLS process spawned (pid={})", child.id());

    let child_stdin: SharedWriter = Arc::new(Mutex::new(Box::new(child.stdin.take().unwrap())));
    let child_stdout = child.stdout.take().unwrap();
    let alive = Arc::new(AtomicBool::new(true));

    let owned_id_prefix = format!("{proxy_id}-proxy-");
    let pending = Arc::new(PendingResponses::new());
    let decompile = DecompileCoordinator::new(
        Arc::clone(&child_stdin),
        Arc::clone(&pending),
        owned_id_prefix.clone(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let port_file = Path::new(workdir).join("proxy").join(&proxy_id);
    fs::create_dir_all(port_file.parent().unwrap()).unwrap();
    fs::write(&port_file, port.to_string()).unwrap();

    lsp_info!("HTTP server listening on 127.0.0.1:{port}");

    let id_counter = Arc::new(AtomicU64::new(1));
    let job_counter = Arc::new(AtomicU64::new(1));

    // Track requests whose responses may contain jdt:// URIs so they can be
    // intercepted and rewritten.
    let tracked_ids: TrackedRequests = Arc::new(Mutex::new(HashMap::new()));
    let active_rewrites: ActiveRewrites = Arc::new(Mutex::new(HashMap::new()));
    let suppressed_responses = Arc::new(Mutex::new(SuppressedResponses::default()));
    let latest_workspace_job = Arc::new(Mutex::new(None::<u64>));

    // --- Thread 1: Zed stdin -> JDTLS stdin ---
    let stdin_context = StdinContext {
        writer: Arc::clone(&child_stdin),
        alive: Arc::clone(&alive),
        tracked: Arc::clone(&tracked_ids),
        active: Arc::clone(&active_rewrites),
        jobs: Arc::clone(&job_counter),
        decompile: decompile.clone(),
        output: output.clone(),
        suppressed: Arc::clone(&suppressed_responses),
        latest_workspace: Arc::clone(&latest_workspace_job),
    };
    thread::spawn(move || run_zed_input(stdin_context));

    // --- Thread 2: JDTLS stdout -> Zed stdout ---
    let stdout_context = StdoutContext {
        pending: Arc::clone(&pending),
        alive: Arc::clone(&alive),
        tracked: Arc::clone(&tracked_ids),
        active: Arc::clone(&active_rewrites),
        decompile: decompile.clone(),
        output: output.clone(),
        suppressed: Arc::clone(&suppressed_responses),
        latest_workspace: Arc::clone(&latest_workspace_job),
    };
    let stdout_thread = thread::spawn(move || run_jdtls_output(child_stdout, stdout_context));

    // --- Thread 3: HTTP server for extension requests ---
    let http_writer = Arc::clone(&child_stdin);
    let http_pending = Arc::clone(&pending);
    let http_alive = Arc::clone(&alive);
    let http_id_counter = Arc::clone(&id_counter);
    let http_owned_id_prefix = owned_id_prefix.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            if !http_alive.load(Ordering::Relaxed) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let writer = Arc::clone(&http_writer);
            let pend = Arc::clone(&http_pending);
            let counter = Arc::clone(&http_id_counter);
            let owned_id_prefix = http_owned_id_prefix.clone();

            thread::spawn(move || {
                handle_http(stream, writer, pend, counter, &owned_id_prefix);
            });
        }
    });

    // --- Thread 4: Parent process monitor ---
    spawn_parent_monitor(Arc::clone(&alive), child.id());

    // Poll so broken editor/JDTLS transport can terminate the child and release
    // all pending proxy jobs instead of waiting indefinitely in `Child::wait`.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if alive.load(Ordering::Relaxed) && !output.failed() => {
                thread::sleep(std::time::Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                break child.wait();
            }
            Err(error) => break Err(error),
        }
    };
    alive.store(false, Ordering::Relaxed);
    let _ = stdout_thread.join();
    lsp_info!("JDTLS process exited: {status:?}");
    pending.clear();
    decompile.shutdown();
    output.shutdown();
    decompile.cleanup_cache();
    let _ = fs::remove_file(&port_file);
}

fn run_zed_input(context: StdinContext) {
    let stdin = io::stdin().lock();
    let mut reader = LspReader::new(BufReader::new(stdin));
    while context.alive.load(Ordering::Relaxed) {
        let raw = match reader.read_message() {
            Ok(Some(raw)) => raw,
            Ok(None) | Err(_) => break,
        };
        let raw = if raw_has_id(&raw) {
            rewrite_request_uri(raw)
        } else {
            raw
        };
        if route_zed_message(&context, &raw) == InputRoute::Forward
            && !write_to_jdtls(&context.writer, &raw)
        {
            break;
        }
    }
    context.alive.store(false, Ordering::Relaxed);
}

fn route_zed_message(context: &StdinContext, raw: &[u8]) -> InputRoute {
    let has_id = raw_has_id(raw);
    if !has_id && !contains_subslice(raw, b"$/cancelRequest") {
        // Editor notifications carry no id. Decompiled-source worktrees must not
        // leak didOpen/didChange/didSave/didClose into JDTLS: those documents
        // live under /tmp/jdtls_decompiled and only exist as a navigation target.
        if contains_subslice(raw, b"jdtls_decompiled") {
            if let Some(message) = parse_lsp_content(raw) {
                if is_decompiled_document_notification(&message) {
                    return InputRoute::Consumed;
                }
            }
        }
        return InputRoute::Forward;
    }
    let Some(message) = parse_lsp_content(raw) else {
        return InputRoute::Forward;
    };
    if message.get("method").and_then(Value::as_str) == Some("$/cancelRequest") {
        return route_zed_cancellation(context, &message);
    }
    if has_id {
        track_zed_request(context, &message);
    }
    InputRoute::Forward
}

/// Rewrite `file://` document URIs that point at a cached decompiled source
/// file back to their original `jdt://` URI before forwarding to JDTLS. This is
/// what makes further navigation *inside* a decompiled class work.
fn rewrite_request_uri(raw: Vec<u8>) -> Vec<u8> {
    if !contains_subslice(&raw, b"jdtls_decompiled") {
        return raw;
    }
    let Some(mut message) = parse_lsp_content(&raw) else {
        return raw;
    };
    let Some(file_uri) = message
        .pointer("/params/textDocument/uri")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return raw;
    };
    let Some(path) = file_uri_to_path(&file_uri) else {
        return raw;
    };
    let Some(jdt_uri) = decompile::jdt_uri_for_cached_file(&path) else {
        return raw;
    };

    *message.pointer_mut("/params/textDocument/uri").unwrap() = Value::String(jdt_uri);
    encode_lsp(&message).into_bytes()
}

/// Whether an editor notification concerns a decompiled-source document whose
/// lifecycle JDTLS must not observe.
fn is_decompiled_document_notification(message: &Value) -> bool {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(
        method,
        "textDocument/didOpen"
            | "textDocument/didChange"
            | "textDocument/didSave"
            | "textDocument/didClose"
    ) {
        return false;
    }

    message
        .pointer("/params/textDocument/uri")
        .and_then(Value::as_str)
        .is_some_and(|uri| uri.contains("jdtls_decompiled"))
}

fn route_zed_cancellation(context: &StdinContext, message: &Value) -> InputRoute {
    let Some(id) = message.pointer("/params/id") else {
        return InputRoute::Forward;
    };
    let already_suppressed = context.suppressed.lock().unwrap().contains(id);
    let (tracked, active_token) =
        take_request_for_cancellation(&context.tracked, &context.active, id);
    if let Some(request) = tracked {
        clear_latest_workspace(&context.latest_workspace, request.token);
    }
    let handled_locally = active_token.is_some();
    if let Some(token) = active_token {
        clear_latest_workspace(&context.latest_workspace, token);
        context.decompile.cancel(token);
        context.output.send_value(&request_canceled(id));
    }

    if handled_locally || already_suppressed {
        InputRoute::Consumed
    } else {
        InputRoute::Forward
    }
}

fn track_zed_request(context: &StdinContext, message: &Value) {
    let token = context.jobs.fetch_add(1, Ordering::Relaxed);
    let Some((id, request)) = tracked_request_for(message, token) else {
        return;
    };
    if request.method == "workspace/symbol" {
        supersede_workspace_job(context, token);
    }

    let (previous, active_token) = {
        let mut tracked = context.tracked.lock().unwrap();
        let mut active = context.active.lock().unwrap();
        let previous = tracked.insert(id.clone(), request);
        let active_token = active.remove(&id);
        (previous, active_token)
    };
    if let Some(previous) = previous {
        clear_latest_workspace(&context.latest_workspace, previous.token);
    }
    if let Some(active_token) = active_token {
        context.decompile.cancel(active_token);
    }
}

fn supersede_workspace_job(context: &StdinContext, token: u64) {
    let previous = context.latest_workspace.lock().unwrap().replace(token);
    let Some(previous) = previous else {
        return;
    };
    let (suppressed, active) = retire_request_token(&context.tracked, &context.active, previous);
    if let Some(id) = suppressed {
        context.suppressed.lock().unwrap().insert(id.clone());
        cancel_jdtls_request(&context.writer, &id);
    }
    if active {
        context.decompile.cancel(previous);
    }
}

fn write_to_jdtls(writer: &SharedWriter, raw: &[u8]) -> bool {
    let mut writer = writer.lock().unwrap();
    writer.write_all(raw).is_ok() && writer.flush().is_ok()
}

fn run_jdtls_output(reader: impl io::Read, context: StdoutContext) {
    let mut reader = LspReader::new(BufReader::new(reader));
    while let Ok(Some(raw)) = reader.read_message() {
        match route_jdtls_message(&context, raw) {
            OutputRoute::Raw(raw) => {
                context.output.send_raw(raw);
            }
            OutputRoute::Value(message) => {
                context.output.send_value(&message);
            }
            OutputRoute::Consumed => {}
        }
    }
    context.alive.store(false, Ordering::Relaxed);
}

fn route_jdtls_message(context: &StdoutContext, raw: Vec<u8>) -> OutputRoute {
    // Notifications cannot be responses that the proxy needs to intercept.
    if !raw_has_id(&raw) {
        return OutputRoute::Raw(raw);
    }
    let Some(message) = parse_lsp_content(&raw) else {
        return OutputRoute::Raw(raw);
    };
    if context.pending.route(&message) {
        return OutputRoute::Consumed;
    }
    if message
        .get("id")
        .is_some_and(|id| context.suppressed.lock().unwrap().take(id))
    {
        return OutputRoute::Consumed;
    }
    if message.get("method").is_some() {
        return OutputRoute::Raw(raw);
    }
    let Some(id) = message.get("id").cloned() else {
        return OutputRoute::Raw(raw);
    };
    let request = context.tracked.lock().unwrap().get(&id).cloned();
    let Some(request) = request else {
        return OutputRoute::Raw(raw);
    };

    route_tracked_response(context, raw, message, id, request)
}

fn route_tracked_response(
    context: &StdoutContext,
    raw: Vec<u8>,
    mut message: Value,
    id: Value,
    request: TrackedRequest,
) -> OutputRoute {
    if let Some(fallback) = completion_resolve_fallback(&message, &request) {
        remove_tracked_request(&context.tracked, &id, request.token);
        if should_log_completion_fallback() {
            lsp_warn!(
                "JDTLS completion resolution failed with -32603; \
                 using the unresolved item, so documentation, imports, \
                 commands, or additional edits may be missing"
            );
        }
        return OutputRoute::Value(fallback);
    }
    if message.get("error").is_some() {
        remove_tracked_request(&context.tracked, &id, request.token);
        clear_latest_workspace(&context.latest_workspace, request.token);
        return OutputRoute::Raw(raw);
    }
    if request.rewrite == RewriteKind::Completion {
        remove_tracked_request(&context.tracked, &id, request.token);
        process_completions(&mut message);
        return OutputRoute::Value(message);
    }

    queue_rewrite_response(context, raw, message, id, request)
}

fn queue_rewrite_response(
    context: &StdoutContext,
    raw: Vec<u8>,
    message: Value,
    id: Value,
    request: TrackedRequest,
) -> OutputRoute {
    let sanitize_completion = request.method == "completionItem/resolve";
    let mode = match request.rewrite {
        RewriteKind::Locations => RewriteMode::Locations,
        RewriteKind::Documentation => RewriteMode::Strings,
        RewriteKind::Completion => unreachable!(),
    };
    let priority = if request.method == "workspace/symbol" {
        Priority::Bulk
    } else {
        Priority::Interactive
    };
    let deadline = std::time::Instant::now() + rewrite_timeout(&request.method);
    let workspace_job = (request.method == "workspace/symbol")
        .then(|| (Arc::clone(&context.latest_workspace), request.token));
    if !activate_rewrite(&context.tracked, &context.active, &id, request.token) {
        return OutputRoute::Raw(raw);
    }
    if context.decompile.is_canceled(request.token) {
        context.active.lock().unwrap().remove(&id);
        context.decompile.consume_cancellation(request.token);
        clear_latest_workspace(&context.latest_workspace, request.token);
        return OutputRoute::Consumed;
    }

    let token = request.token;
    let output = context.output.clone();
    let active = Arc::clone(&context.active);
    let completion_coordinator = context.decompile.clone();
    let job = RewriteJob {
        token,
        message,
        mode,
        priority,
        deadline,
        complete: Box::new(move |mut message| {
            if sanitize_completion {
                sanitize_resolved_completion(&mut message);
            }
            let owns_response = active.lock().unwrap().remove(&id) == Some(token);
            completion_coordinator.consume_cancellation(token);
            if owns_response {
                output.send_value(&message);
            }
            if let Some((latest, token)) = workspace_job {
                clear_latest_workspace(&latest, token);
            }
        }),
    };
    if let Err(job) = context.decompile.submit(job) {
        let RewriteJob {
            message, complete, ..
        } = job;
        complete(message);
    }
    OutputRoute::Consumed
}

// --- Utilities ---

fn hex_encode(s: &str) -> String {
    hex::encode(Sha1::digest(s.as_bytes()))
}

fn clear_latest_workspace(latest: &Mutex<Option<u64>>, token: u64) {
    let mut latest = latest.lock().unwrap();
    if *latest == Some(token) {
        *latest = None;
    }
}

fn request_canceled(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32800,
            "message": "Request cancelled."
        }
    })
}

fn cancel_jdtls_request(writer: &SharedWriter, id: &Value) {
    let cancel = encode_lsp(&json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": id }
    }));
    let mut writer = writer.lock().unwrap();
    let _ = writer.write_all(cancel.as_bytes());
    let _ = writer.flush();
}

fn take_request_for_cancellation(
    tracked: &Mutex<HashMap<Value, TrackedRequest>>,
    active: &Mutex<HashMap<Value, u64>>,
    id: &Value,
) -> (Option<TrackedRequest>, Option<u64>) {
    let mut tracked = tracked.lock().unwrap();
    let mut active = active.lock().unwrap();
    (tracked.remove(id), active.remove(id))
}

fn retire_request_token(
    tracked: &Mutex<HashMap<Value, TrackedRequest>>,
    active: &Mutex<HashMap<Value, u64>>,
    token: u64,
) -> (Option<Value>, bool) {
    let mut tracked = tracked.lock().unwrap();
    let mut active = active.lock().unwrap();
    let tracked_id = tracked
        .iter()
        .find_map(|(id, request)| (request.token == token).then(|| id.clone()));
    if let Some(id) = &tracked_id {
        tracked.remove(id);
    }
    let active_id = active
        .iter()
        .find_map(|(id, active_token)| (*active_token == token).then(|| id.clone()));
    if let Some(id) = &active_id {
        active.remove(id);
    }
    (tracked_id, active_id.is_some())
}

fn activate_rewrite(
    tracked: &Mutex<HashMap<Value, TrackedRequest>>,
    active: &Mutex<HashMap<Value, u64>>,
    id: &Value,
    token: u64,
) -> bool {
    let mut tracked = tracked.lock().unwrap();
    let mut active = active.lock().unwrap();
    if tracked.get(id).map(|request| request.token) != Some(token) {
        return false;
    }
    tracked.remove(id);
    active.insert(id.clone(), token);
    true
}

fn remove_tracked_request(tracked: &Mutex<HashMap<Value, TrackedRequest>>, id: &Value, token: u64) {
    let mut tracked = tracked.lock().unwrap();
    if tracked.get(id).map(|request| request.token) == Some(token) {
        tracked.remove(id);
    }
}

fn should_log_completion_fallback() -> bool {
    static LAST_WARNING: OnceLock<Mutex<Option<std::time::Instant>>> = OnceLock::new();
    let now = std::time::Instant::now();
    let mut last = LAST_WARNING
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap();
    if last
        .is_some_and(|previous| now.duration_since(previous) < std::time::Duration::from_secs(60))
    {
        return false;
    }
    *last = Some(now);
    true
}

fn tracked_request_for(msg: &Value, token: u64) -> Option<(Value, TrackedRequest)> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?.clone();
    let rewrite = match method {
        "textDocument/completion" => RewriteKind::Completion,
        "textDocument/definition"
        | "textDocument/declaration"
        | "textDocument/typeDefinition"
        | "textDocument/implementation"
        | "textDocument/references"
        | "textDocument/prepareCallHierarchy"
        | "callHierarchy/incomingCalls"
        | "callHierarchy/outgoingCalls"
        | "textDocument/prepareTypeHierarchy"
        | "typeHierarchy/supertypes"
        | "typeHierarchy/subtypes"
        | "workspace/symbol" => RewriteKind::Locations,
        "textDocument/hover" | "textDocument/signatureHelp" | "completionItem/resolve" => {
            RewriteKind::Documentation
        }
        _ => return None,
    };

    let original_params = if method == "completionItem/resolve" {
        msg.get("params")
            .filter(|params| params.is_object())
            .cloned()
    } else {
        None
    };

    Some((
        id,
        TrackedRequest::new(token, method, rewrite, original_params),
    ))
}

fn completion_resolve_fallback(message: &Value, request: &TrackedRequest) -> Option<Value> {
    if request.method != "completionItem/resolve"
        || message.pointer("/error/code").and_then(Value::as_i64) != Some(-32603)
    {
        return None;
    }
    let item = request.original_params.as_ref()?;
    Some(json!({
        "jsonrpc": "2.0",
        "id": message.get("id")?.clone(),
        "result": item,
    }))
}

fn rewrite_timeout(method: &str) -> std::time::Duration {
    match method {
        "textDocument/hover" | "textDocument/signatureHelp" | "completionItem/resolve" => {
            std::time::Duration::from_millis(500)
        }
        "workspace/symbol" => std::time::Duration::from_secs(1),
        _ => std::time::Duration::from_secs(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct RoutingFixture {
        writer: SharedWriter,
        alive: Arc<AtomicBool>,
        jobs: Arc<AtomicU64>,
        pending: Arc<PendingResponses>,
        tracked: TrackedRequests,
        active: ActiveRewrites,
        suppressed: SharedSuppressedResponses,
        latest_workspace: LatestWorkspaceJob,
        decompile: DecompileCoordinator,
        output: Output,
    }

    impl RoutingFixture {
        fn new() -> Self {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Vec::<u8>::new())));
            let pending = Arc::new(PendingResponses::new());
            let decompile = DecompileCoordinator::new(
                Arc::clone(&writer),
                Arc::clone(&pending),
                "routing-test-".to_string(),
            );
            Self {
                writer,
                alive: Arc::new(AtomicBool::new(true)),
                jobs: Arc::new(AtomicU64::new(1)),
                pending,
                tracked: Arc::new(Mutex::new(HashMap::new())),
                active: Arc::new(Mutex::new(HashMap::new())),
                suppressed: Arc::new(Mutex::new(SuppressedResponses::default())),
                latest_workspace: Arc::new(Mutex::new(None)),
                decompile,
                output: Output::start(),
            }
        }

        fn stdin_context(&self) -> StdinContext {
            StdinContext {
                writer: Arc::clone(&self.writer),
                alive: Arc::clone(&self.alive),
                tracked: Arc::clone(&self.tracked),
                active: Arc::clone(&self.active),
                jobs: Arc::clone(&self.jobs),
                decompile: self.decompile.clone(),
                output: self.output.clone(),
                suppressed: Arc::clone(&self.suppressed),
                latest_workspace: Arc::clone(&self.latest_workspace),
            }
        }

        fn stdout_context(&self) -> StdoutContext {
            StdoutContext {
                pending: Arc::clone(&self.pending),
                alive: Arc::clone(&self.alive),
                tracked: Arc::clone(&self.tracked),
                active: Arc::clone(&self.active),
                decompile: self.decompile.clone(),
                output: self.output.clone(),
                suppressed: Arc::clone(&self.suppressed),
                latest_workspace: Arc::clone(&self.latest_workspace),
            }
        }
    }

    impl Drop for RoutingFixture {
        fn drop(&mut self) {
            self.decompile.shutdown();
            self.output.shutdown();
        }
    }

    fn frame(value: &Value) -> Vec<u8> {
        encode_lsp(value).into_bytes()
    }

    #[test]
    fn stdin_router_tracks_requests_and_forwards_cancellation_before_rewrite() {
        let fixture = RoutingFixture::new();
        let context = fixture.stdin_context();
        let request = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "textDocument/definition",
            "params": {}
        });

        assert_eq!(
            route_zed_message(&context, &frame(&request)),
            InputRoute::Forward
        );
        assert_eq!(
            fixture.tracked.lock().unwrap().get(&json!(7)),
            Some(&TrackedRequest::new(
                1,
                "textDocument/definition",
                RewriteKind::Locations,
                None
            ))
        );

        let cancellation = json!({
            "jsonrpc": "2.0",
            "method": "$/cancelRequest",
            "params": { "id": 7 }
        });
        assert_eq!(
            route_zed_message(&context, &frame(&cancellation)),
            InputRoute::Forward
        );
        assert!(fixture.tracked.lock().unwrap().is_empty());
    }

    #[test]
    fn rewrites_decompiled_file_request_uri() {
        let dir = std::env::temp_dir().join("jdtls_decompiled/rewrite-test");
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("String.java");
        fs::write(&target, "class String {}").unwrap();
        fs::write(
            format!("{}.jdt-uri", target.display()),
            "jdt://contents/java.base/java.lang/String.class",
        )
        .unwrap();

        let file_uri = proxy_common::path_to_file_uri(&target);
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": file_uri },
                "position": { "line": 0, "character": 0 }
            }
        });

        let rewritten = rewrite_request_uri(frame(&request));
        let parsed = parse_lsp_content(&rewritten).unwrap();
        assert_eq!(
            parsed["params"]["textDocument"]["uri"],
            json!("jdt://contents/java.base/java.lang/String.class")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn decompiled_document_notifications_are_recognized() {
        assert!(is_decompiled_document_notification(&json!({
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///tmp/jdtls_decompiled/session_x/uri_y/String.java"
                }
            }
        })));
        assert!(!is_decompiled_document_notification(&json!({
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///workspace/Foo.java"
                }
            }
        })));
        assert!(!is_decompiled_document_notification(&json!({
            "method": "textDocument/publishDiagnostics",
            "params": {
                "textDocument": {
                    "uri": "file:///tmp/jdtls_decompiled/session_x/uri_y/String.java"
                }
            }
        })));
    }

    #[test]
    fn suppresses_decompiled_document_notifications() {
        let fixture = RoutingFixture::new();
        let context = fixture.stdin_context();
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///tmp/jdtls_decompiled/session_x/uri_y/String.java"
                }
            }
        });

        assert_eq!(
            route_zed_message(&context, &frame(&notification)),
            InputRoute::Consumed
        );
    }

    #[test]
    fn stdout_router_returns_processed_completion_values() {
        let fixture = RoutingFixture::new();
        let context = fixture.stdout_context();
        fixture.tracked.lock().unwrap().insert(
            json!(9),
            TrackedRequest::new(1, "textDocument/completion", RewriteKind::Completion, None),
        );
        let response = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "result": [{
                "kind": 15,
                "textEditText": "$TM_SELECTED_TEXT.field"
            }]
        });

        assert_eq!(
            route_jdtls_message(&context, frame(&response)),
            OutputRoute::Value(json!({
                "jsonrpc": "2.0",
                "id": 9,
                "result": [{
                    "kind": 15,
                    "textEditText": ".field"
                }]
            }))
        );
        assert!(fixture.tracked.lock().unwrap().is_empty());
    }

    #[test]
    fn stdout_router_consumes_pending_and_suppressed_responses() {
        let fixture = RoutingFixture::new();
        let context = fixture.stdout_context();
        let pending_id = json!("proxy-request");
        let receiver = fixture.pending.register(pending_id.clone());
        let pending_response = json!({ "jsonrpc": "2.0", "id": pending_id, "result": "ok" });

        assert_eq!(
            route_jdtls_message(&context, frame(&pending_response)),
            OutputRoute::Consumed
        );
        assert_eq!(receiver.recv().unwrap(), pending_response);

        let suppressed_id = json!("superseded-request");
        fixture
            .suppressed
            .lock()
            .unwrap()
            .insert(suppressed_id.clone());
        let suppressed_response = json!({ "jsonrpc": "2.0", "id": suppressed_id, "result": null });
        assert_eq!(
            route_jdtls_message(&context, frame(&suppressed_response)),
            OutputRoute::Consumed
        );
        assert!(!fixture
            .suppressed
            .lock()
            .unwrap()
            .contains(&json!("superseded-request")));
    }

    #[test]
    fn stdio_routers_preserve_unhandled_raw_frames() {
        let fixture = RoutingFixture::new();
        let stdin_context = fixture.stdin_context();
        let stdout_context = fixture.stdout_context();
        let malformed = b"Content-Length: 6\r\n\r\n{\"id\":".to_vec();

        assert_eq!(
            route_zed_message(&stdin_context, &malformed),
            InputRoute::Forward
        );
        assert_eq!(
            route_jdtls_message(&stdout_context, malformed.clone()),
            OutputRoute::Raw(malformed)
        );
    }

    #[test]
    fn tracks_location_response_methods() {
        let methods = [
            "textDocument/definition",
            "textDocument/declaration",
            "textDocument/typeDefinition",
            "textDocument/implementation",
            "textDocument/references",
            "textDocument/prepareCallHierarchy",
            "callHierarchy/incomingCalls",
            "callHierarchy/outgoingCalls",
            "textDocument/prepareTypeHierarchy",
            "typeHierarchy/supertypes",
            "typeHierarchy/subtypes",
            "workspace/symbol",
        ];

        for method in methods {
            let request = json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": method,
                "params": {}
            });

            assert_eq!(
                tracked_request_for(&request, 42),
                Some((
                    json!(7),
                    TrackedRequest::new(42, method, RewriteKind::Locations, None)
                )),
                "{method} should rewrite location URIs"
            );
        }
    }

    #[test]
    fn tracks_completion_resolve_documentation() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "resolve-1",
            "method": "completionItem/resolve",
            "params": { "label": "String" }
        });

        assert_eq!(
            tracked_request_for(&request, 43),
            Some((
                json!("resolve-1"),
                TrackedRequest::new(
                    43,
                    "completionItem/resolve",
                    RewriteKind::Documentation,
                    Some(json!({ "label": "String" }))
                )
            ))
        );
    }

    #[test]
    fn ignores_untracked_requests_and_notifications() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "textDocument/rename",
            "params": {}
        });
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {}
        });

        assert_eq!(tracked_request_for(&request, 1), None);
        assert_eq!(tracked_request_for(&notification, 2), None);
    }

    #[test]
    fn tracks_completion_responses_by_method() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "textDocument/completion",
            "params": {}
        });

        assert_eq!(
            tracked_request_for(&request, 44),
            Some((
                json!(9),
                TrackedRequest::new(44, "textDocument/completion", RewriteKind::Completion, None)
            ))
        );
    }

    #[test]
    fn activating_rewrite_moves_request_atomically() {
        let id = json!(9);
        let tracked = Mutex::new(HashMap::from([(
            id.clone(),
            TrackedRequest::new(44, "textDocument/definition", RewriteKind::Locations, None),
        )]));
        let active = Mutex::new(HashMap::new());

        assert!(activate_rewrite(&tracked, &active, &id, 44));
        assert!(tracked.lock().unwrap().is_empty());
        assert_eq!(active.lock().unwrap().get(&id), Some(&44));
    }

    #[test]
    fn retiring_tracked_request_returns_id_for_late_suppression() {
        let id = json!("workspace-1");
        let tracked = Mutex::new(HashMap::from([(
            id.clone(),
            TrackedRequest::new(45, "workspace/symbol", RewriteKind::Locations, None),
        )]));
        let active = Mutex::new(HashMap::new());

        assert_eq!(
            retire_request_token(&tracked, &active, 45),
            (Some(id), false)
        );
        assert!(tracked.lock().unwrap().is_empty());
    }

    #[test]
    fn suppressed_response_ids_remain_owned_until_the_response_arrives() {
        let mut suppressed = SuppressedResponses::default();
        for id in 0..=1024 {
            suppressed.insert(json!(id));
        }

        assert!(suppressed.contains(&json!(0)));
        assert!(suppressed.take(&json!(0)));
        assert!(!suppressed.contains(&json!(0)));
        assert!(suppressed.take(&json!(1024)));
    }

    #[test]
    fn cancellation_response_preserves_request_id() {
        assert_eq!(
            request_canceled(&json!("request-1")),
            json!({
                "jsonrpc": "2.0",
                "id": "request-1",
                "error": {
                    "code": -32800,
                    "message": "Request cancelled."
                }
            })
        );
    }

    #[test]
    fn cancellation_removes_unanswered_tracking_state() {
        let id = json!("hover-1");
        let tracked = Mutex::new(HashMap::from([(
            id.clone(),
            TrackedRequest::new(46, "textDocument/hover", RewriteKind::Documentation, None),
        )]));
        let active = Mutex::new(HashMap::new());

        let (request, active_token) = take_request_for_cancellation(&tracked, &active, &id);

        assert_eq!(request.map(|request| request.token), Some(46));
        assert_eq!(active_token, None);
        assert!(tracked.lock().unwrap().is_empty());
    }

    #[test]
    fn falls_back_only_for_internal_completion_resolve_errors() {
        let request = TrackedRequest::new(
            1,
            "completionItem/resolve",
            RewriteKind::Documentation,
            Some(json!({ "label": "value", "kind": 6 })),
        );
        let internal = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "error": { "code": -32603, "message": "Invalid completion proposal" }
        });
        let canceled = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "error": { "code": -32800, "message": "Request cancelled" }
        });

        assert_eq!(
            completion_resolve_fallback(&internal, &request),
            Some(json!({
                "jsonrpc": "2.0",
                "id": 5,
                "result": { "label": "value", "kind": 6 }
            }))
        );
        assert_eq!(completion_resolve_fallback(&canceled, &request), None);
    }

    #[test]
    fn malformed_completion_resolve_params_do_not_fallback() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "completionItem/resolve",
            "params": null
        });
        let (_, tracked) = tracked_request_for(&request, 1).unwrap();
        let error = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "error": { "code": -32603, "message": "Internal error" }
        });

        assert_eq!(completion_resolve_fallback(&error, &tracked), None);
    }

    #[test]
    fn proxy_id_stays_under_macos_filename_limit() {
        let long_workspace = format!("/Users/test/{}", "a".repeat(200));

        let id = hex_encode(&long_workspace);

        assert!(
            id.len() <= 255,
            "proxy_id exceeded macOS filename limit: {} bytes",
            id.len()
        );
        assert_eq!(id.len(), 40, "SHA-1 hex should always be 40 chars");
    }

    #[test]
    fn proxy_port_file_works_for_long_workdir() {
        let temp = std::env::temp_dir().join("java-proxy-long-path-test");
        let _ = fs::remove_dir_all(&temp);
        let long_workspace = temp.join("a".repeat(200));
        fs::create_dir_all(&long_workspace).unwrap();

        let proxy_id = hex_encode(long_workspace.to_string_lossy().trim_end_matches('/'));
        let port_file = long_workspace.join("proxy").join(&proxy_id);

        fs::create_dir_all(port_file.parent().unwrap()).unwrap();
        fs::write(&port_file, "3000").unwrap();

        assert!(port_file.file_name().unwrap().len() <= 255);
    }
}
