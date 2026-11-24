use crate::{lsp_error, pending::PendingResponses};
use proxy_common::{encode_lsp, path_to_file_uri};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const DECOMPILED_DIR: &str = "jdtls_decompiled";
const FETCH_WORKERS: usize = 2;
const JOB_WORKERS: usize = 2;
const MAX_QUEUED_JOBS: usize = 64;
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(2);

pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewriteMode {
    Locations,
    Strings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Interactive,
    Bulk,
}

pub struct RewriteJob {
    pub token: u64,
    pub message: Value,
    pub mode: RewriteMode,
    pub priority: Priority,
    pub deadline: Instant,
    pub complete: Box<dyn FnOnce(Value) + Send>,
}

#[derive(Clone)]
pub struct DecompileCoordinator {
    inner: Arc<Inner>,
}

struct Inner {
    fetcher: Arc<dyn ClassContentFetcher>,
    owned_id_prefix: String,
    cache_dir: PathBuf,
    request_counter: AtomicU64,
    max_bulk_jobs: usize,
    state: Mutex<State>,
    work_available: Condvar,
    state_changed: Condvar,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.cache_dir);
    }
}

#[derive(Default)]
struct State {
    uris: HashMap<String, UriEntry>,
    interactive_uris: VecDeque<String>,
    bulk_uris: VecDeque<String>,
    interactive_jobs: VecDeque<RewriteJob>,
    bulk_jobs: VecDeque<RewriteJob>,
    canceled_jobs: HashSet<u64>,
    bulk_jobs_active: usize,
    bulk_fetches: usize,
    shutdown: bool,
}

struct UriEntry {
    status: UriStatus,
    waiters: HashSet<u64>,
}

enum UriStatus {
    Queued(Priority),
    InFlight { request_id: Value },
    Ready(String),
    Failed(Instant),
}

trait ClassContentFetcher: Send + Sync {
    fn fetch(&self, uri: &str, request_id: Value) -> Option<String>;
    fn cancel(&self, request_id: &Value);
}

struct JdtlsFetcher {
    writer: SharedWriter,
    pending: Arc<PendingResponses>,
}

impl ClassContentFetcher for JdtlsFetcher {
    fn fetch(&self, uri: &str, request_id: Value) -> Option<String> {
        let receiver = self.pending.register(request_id.clone());
        let request = encode_lsp(&json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "java/classFileContents",
            "params": { "uri": uri }
        }));

        let write_succeeded = {
            let mut writer = self.writer.lock().unwrap();
            writer.write_all(request.as_bytes()).is_ok() && writer.flush().is_ok()
        };
        if !write_succeeded {
            self.pending.remove(&request_id);
            return None;
        }

        match receiver.recv_timeout(FETCH_TIMEOUT) {
            Ok(response) => response
                .get("result")
                .and_then(Value::as_str)
                .filter(|content| !content.is_empty())
                .map(str::to_string),
            Err(_) => {
                self.pending.remove(&request_id);
                None
            }
        }
    }

    fn cancel(&self, request_id: &Value) {
        self.pending.remove(request_id);
        let cancel = encode_lsp(&json!({
            "jsonrpc": "2.0",
            "method": "$/cancelRequest",
            "params": { "id": request_id }
        }));
        let mut writer = self.writer.lock().unwrap();
        let _ = writer.write_all(cancel.as_bytes());
        let _ = writer.flush();
    }
}

impl DecompileCoordinator {
    pub fn new(
        writer: SharedWriter,
        pending: Arc<PendingResponses>,
        owned_id_prefix: String,
    ) -> Self {
        Self::with_fetcher(
            Arc::new(JdtlsFetcher { writer, pending }),
            owned_id_prefix,
            FETCH_WORKERS,
            JOB_WORKERS,
        )
    }

    fn with_fetcher(
        fetcher: Arc<dyn ClassContentFetcher>,
        owned_id_prefix: String,
        fetch_workers: usize,
        job_workers: usize,
    ) -> Self {
        let cache_dir = session_cache_dir(&owned_id_prefix);
        let coordinator = Self {
            inner: Arc::new(Inner {
                fetcher,
                owned_id_prefix,
                cache_dir,
                request_counter: AtomicU64::new(1),
                max_bulk_jobs: job_workers.saturating_sub(1).max(1),
                state: Mutex::new(State::default()),
                work_available: Condvar::new(),
                state_changed: Condvar::new(),
                workers: Mutex::new(Vec::new()),
            }),
        };

        let mut workers = Vec::with_capacity(fetch_workers + job_workers + 1);
        for _ in 0..fetch_workers {
            let inner = Arc::clone(&coordinator.inner);
            workers.push(thread::spawn(move || fetch_worker(inner)));
        }
        for _ in 0..job_workers {
            let inner = Arc::clone(&coordinator.inner);
            workers.push(thread::spawn(move || job_worker(inner)));
        }
        let inner = Arc::clone(&coordinator.inner);
        workers.push(thread::spawn(move || deadline_worker(inner)));
        *coordinator.inner.workers.lock().unwrap() = workers;

        coordinator
    }

    /// Enqueues without blocking the JDTLS stdout router. On saturation the
    /// original job is returned so the caller can forward it unchanged.
    pub fn submit(&self, job: RewriteJob) -> Result<(), RewriteJob> {
        let mut state = self.inner.state.lock().unwrap();
        if state.shutdown || state.interactive_jobs.len() + state.bulk_jobs.len() >= MAX_QUEUED_JOBS
        {
            return Err(job);
        }

        if job.priority == Priority::Bulk {
            state.bulk_jobs.push_back(job);
        } else {
            state.interactive_jobs.push_back(job);
        }
        self.inner.work_available.notify_all();
        Ok(())
    }

    pub fn cancel(&self, token: u64) {
        let request_ids = {
            let mut state = self.inner.state.lock().unwrap();
            if cancel_job_locked(&mut state, token) {
                state.canceled_jobs.remove(&token);
            }
            detach_orphaned_fetches(&mut state)
        };
        for request_id in request_ids {
            self.inner.fetcher.cancel(&request_id);
        }
        self.inner.state_changed.notify_all();
        self.inner.work_available.notify_all();
    }

    pub fn consume_cancellation(&self, token: u64) -> bool {
        self.inner
            .state
            .lock()
            .unwrap()
            .canceled_jobs
            .remove(&token)
    }

    pub fn is_canceled(&self, token: u64) -> bool {
        self.inner
            .state
            .lock()
            .unwrap()
            .canceled_jobs
            .contains(&token)
    }

    pub fn shutdown(&self) {
        let queued = {
            let mut state = self.inner.state.lock().unwrap();
            if state.shutdown {
                Vec::new()
            } else {
                state.shutdown = true;
                let mut queued: Vec<_> = state.interactive_jobs.drain(..).collect();
                queued.extend(state.bulk_jobs.drain(..));
                queued
            }
        };
        self.inner.work_available.notify_all();
        self.inner.state_changed.notify_all();

        for job in queued {
            if !self.consume_cancellation(job.token) {
                (job.complete)(job.message);
            }
        }

        let workers = std::mem::take(&mut *self.inner.workers.lock().unwrap());
        for worker in workers {
            let _ = worker.join();
        }
    }

    pub fn cleanup_cache(&self) {
        let _ = fs::remove_dir_all(&self.inner.cache_dir);
    }
}

fn cancel_job_locked(state: &mut State, token: u64) -> bool {
    state.canceled_jobs.insert(token);
    let queued_jobs = state.interactive_jobs.len() + state.bulk_jobs.len();
    state
        .interactive_jobs
        .retain(|queued| queued.token != token);
    state.bulk_jobs.retain(|queued| queued.token != token);
    for entry in state.uris.values_mut() {
        entry.waiters.remove(&token);
    }
    queued_jobs != state.interactive_jobs.len() + state.bulk_jobs.len()
}

fn detach_orphaned_fetches(state: &mut State) -> Vec<Value> {
    let now = Instant::now();
    let orphaned: Vec<_> = state
        .uris
        .iter()
        .filter(|(_, entry)| entry.waiters.is_empty())
        .filter(|(_, entry)| {
            !matches!(entry.status, UriStatus::Failed(retry_after) if retry_after > now)
        })
        .map(|(uri, _)| uri.clone())
        .collect();
    let mut request_ids = Vec::new();
    for uri in orphaned {
        if let Some(UriEntry {
            status: UriStatus::InFlight { request_id },
            ..
        }) = state.uris.remove(&uri)
        {
            request_ids.push(request_id);
        }
    }
    request_ids
}

fn job_worker(inner: Arc<Inner>) {
    loop {
        let job = {
            let mut state = inner.state.lock().unwrap();
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(job) = state.interactive_jobs.pop_front() {
                    break job;
                }
                if state.bulk_jobs_active < inner.max_bulk_jobs {
                    if let Some(job) = state.bulk_jobs.pop_front() {
                        state.bulk_jobs_active += 1;
                        break job;
                    }
                }
                state = inner.work_available.wait(state).unwrap();
            }
        };

        if is_canceled(&inner, job.token) {
            finish_job(&inner, job.token, job.priority);
            continue;
        }

        let mut message = job.message;
        if Instant::now() < job.deadline {
            rewrite_message(
                &inner,
                job.token,
                &mut message,
                job.mode,
                job.priority,
                job.deadline,
            );
        }
        let canceled = is_canceled(&inner, job.token);
        finish_job(&inner, job.token, job.priority);
        if !canceled {
            (job.complete)(message);
        }
    }
}

fn deadline_worker(inner: Arc<Inner>) {
    loop {
        let expired = {
            let mut state = inner.state.lock().unwrap();
            loop {
                if state.shutdown {
                    return;
                }

                let now = Instant::now();
                let mut expired = take_expired_jobs(&mut state.interactive_jobs, now);
                expired.extend(take_expired_jobs(&mut state.bulk_jobs, now));
                if !expired.is_empty() {
                    break expired;
                }

                let next_deadline = state
                    .interactive_jobs
                    .iter()
                    .chain(state.bulk_jobs.iter())
                    .map(|job| job.deadline)
                    .min();
                state = if let Some(deadline) = next_deadline {
                    let timeout = deadline.saturating_duration_since(Instant::now());
                    inner.work_available.wait_timeout(state, timeout).unwrap().0
                } else {
                    inner.work_available.wait(state).unwrap()
                };
            }
        };

        for job in expired {
            (job.complete)(job.message);
        }
    }
}

fn take_expired_jobs(queue: &mut VecDeque<RewriteJob>, now: Instant) -> Vec<RewriteJob> {
    let mut expired = Vec::new();
    let queued = queue.len();
    for _ in 0..queued {
        let job = queue.pop_front().unwrap();
        if job.deadline <= now {
            expired.push(job);
        } else {
            queue.push_back(job);
        }
    }
    expired
}

fn is_canceled(inner: &Inner, token: u64) -> bool {
    inner.state.lock().unwrap().canceled_jobs.contains(&token)
}

fn finish_job(inner: &Inner, token: u64, priority: Priority) {
    let mut state = inner.state.lock().unwrap();
    if priority == Priority::Bulk {
        state.bulk_jobs_active = state.bulk_jobs_active.saturating_sub(1);
    }
    state.canceled_jobs.remove(&token);
    for entry in state.uris.values_mut() {
        entry.waiters.remove(&token);
    }
    let now = Instant::now();
    state.uris.retain(|_, entry| {
        !entry.waiters.is_empty()
            || matches!(entry.status, UriStatus::InFlight { .. })
            || matches!(entry.status, UriStatus::Failed(retry_after) if retry_after > now)
    });
    inner.work_available.notify_all();
}

fn rewrite_message(
    inner: &Inner,
    token: u64,
    message: &mut Value,
    mode: RewriteMode,
    priority: Priority,
    deadline: Instant,
) {
    let Some(result) = message.get_mut("result") else {
        return;
    };

    let mut uris = Vec::new();
    match mode {
        RewriteMode::Locations => collect_jdt_location_uris(result, &mut uris, &mut HashSet::new()),
        RewriteMode::Strings => collect_jdt_uris(result, &mut uris, &mut HashSet::new()),
    }
    if uris.is_empty() {
        return;
    }

    let replacements = resolve_uris(inner, token, &uris, priority, deadline);
    match mode {
        RewriteMode::Locations => {
            replace_jdt_location_uris(result, &replacements);
        }
        RewriteMode::Strings => replace_in_strings(result, &replacements),
    }
}

fn resolve_uris(
    inner: &Inner,
    token: u64,
    uris: &[String],
    priority: Priority,
    deadline: Instant,
) -> HashMap<String, String> {
    let mut replacements = HashMap::new();
    let mut unresolved = Vec::new();

    for uri in uris {
        let path = cache_path_in(&inner.cache_dir, uri);
        if path.is_file() {
            replacements.insert(uri.clone(), path_to_file_uri(&path));
        } else {
            unresolved.push(uri.clone());
        }
    }
    if unresolved.is_empty() {
        return replacements;
    }

    let mut state = inner.state.lock().unwrap();
    let now = Instant::now();
    state.uris.retain(|_, entry| {
        !entry.waiters.is_empty()
            || matches!(entry.status, UriStatus::InFlight { .. })
            || matches!(entry.status, UriStatus::Failed(retry_after) if retry_after > now)
    });
    if state.canceled_jobs.contains(&token) {
        return replacements;
    }

    for uri in &unresolved {
        let mut enqueue = None;
        match state.uris.get_mut(uri) {
            Some(entry) => {
                entry.waiters.insert(token);
                match entry.status {
                    UriStatus::Ready(ref file_uri) => {
                        replacements.insert(uri.clone(), file_uri.clone());
                    }
                    UriStatus::Failed(retry_after) if retry_after <= Instant::now() => {
                        entry.status = UriStatus::Queued(priority);
                        enqueue = Some(priority);
                    }
                    UriStatus::Queued(Priority::Bulk) if priority == Priority::Interactive => {
                        entry.status = UriStatus::Queued(Priority::Interactive);
                        enqueue = Some(Priority::Interactive);
                    }
                    _ => {}
                }
            }
            None => {
                state.uris.insert(
                    uri.clone(),
                    UriEntry {
                        status: UriStatus::Queued(priority),
                        waiters: HashSet::from([token]),
                    },
                );
                enqueue = Some(priority);
            }
        }
        if let Some(priority) = enqueue {
            queue_uri(&mut state, uri.clone(), priority);
        }
    }
    inner.work_available.notify_all();

    loop {
        let mut waiting = false;
        for uri in &unresolved {
            match state.uris.get(uri).map(|entry| &entry.status) {
                Some(UriStatus::Ready(file_uri)) => {
                    replacements.insert(uri.clone(), file_uri.clone());
                }
                Some(UriStatus::Queued(_) | UriStatus::InFlight { .. }) => waiting = true,
                Some(UriStatus::Failed(_)) | None => {}
            }
        }

        if !waiting
            || state.shutdown
            || state.canceled_jobs.contains(&token)
            || Instant::now() >= deadline
        {
            break;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let (next_state, _) = inner.state_changed.wait_timeout(state, remaining).unwrap();
        state = next_state;
    }

    for uri in &unresolved {
        if let Some(entry) = state.uris.get_mut(uri) {
            entry.waiters.remove(&token);
        }
    }
    let orphaned = detach_orphaned_fetches(&mut state);
    drop(state);
    for request_id in orphaned {
        inner.fetcher.cancel(&request_id);
    }

    replacements
}

fn queue_uri(state: &mut State, uri: String, priority: Priority) {
    match priority {
        Priority::Interactive => state.interactive_uris.push_back(uri),
        Priority::Bulk => state.bulk_uris.push_back(uri),
    }
}

fn fetch_worker(inner: Arc<Inner>) {
    loop {
        let Some((uri, priority, request_id)) = take_uri_work(&inner) else {
            return;
        };

        let content = inner.fetcher.fetch(&uri, request_id.clone());
        let resolved = content
            .and_then(|content| write_cached_source(&inner.cache_dir, &uri, content.as_bytes()));

        let mut state = inner.state.lock().unwrap();
        if priority == Priority::Bulk {
            state.bulk_fetches = state.bulk_fetches.saturating_sub(1);
        }
        if let Some(entry) = state.uris.get_mut(&uri) {
            let is_current = matches!(
                &entry.status,
                UriStatus::InFlight {
                    request_id: current,
                    ..
                } if current == &request_id
            );
            if is_current {
                entry.status = match resolved {
                    Some(file_uri) => UriStatus::Ready(file_uri),
                    None => UriStatus::Failed(Instant::now() + NEGATIVE_CACHE_TTL),
                };
            }
        }
        let remove_ready = state.uris.get(&uri).is_some_and(|entry| {
            entry.waiters.is_empty() && matches!(entry.status, UriStatus::Ready(_))
        });
        if remove_ready {
            state.uris.remove(&uri);
        }
        inner.state_changed.notify_all();
        inner.work_available.notify_all();
    }
}

fn take_uri_work(inner: &Inner) -> Option<(String, Priority, Value)> {
    let mut state = inner.state.lock().unwrap();
    loop {
        if state.shutdown {
            return None;
        }

        let candidate = pop_valid_uri(&mut state, Priority::Interactive).or_else(|| {
            if state.bulk_fetches == 0 {
                pop_valid_uri(&mut state, Priority::Bulk)
            } else {
                None
            }
        });

        if let Some((uri, priority)) = candidate {
            let sequence = inner.request_counter.fetch_add(1, Ordering::Relaxed);
            let request_id =
                Value::String(format!("{}decompile-{sequence}", inner.owned_id_prefix));
            if priority == Priority::Bulk {
                state.bulk_fetches += 1;
            }
            if let Some(entry) = state.uris.get_mut(&uri) {
                entry.status = UriStatus::InFlight {
                    request_id: request_id.clone(),
                };
            }
            return Some((uri, priority, request_id));
        }

        state = inner.work_available.wait(state).unwrap();
    }
}

fn pop_valid_uri(state: &mut State, priority: Priority) -> Option<(String, Priority)> {
    let queue = match priority {
        Priority::Interactive => &mut state.interactive_uris,
        Priority::Bulk => &mut state.bulk_uris,
    };
    while let Some(uri) = queue.pop_front() {
        let valid = state.uris.get(&uri).is_some_and(|entry| {
            !entry.waiters.is_empty()
                && matches!(entry.status, UriStatus::Queued(current) if current == priority)
        });
        if valid {
            return Some((uri, priority));
        }
    }
    None
}

fn session_cache_dir(scope: &str) -> PathBuf {
    static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
    let mut sequence = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let scope = hex::encode(Sha1::digest(scope.as_bytes()));
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = env::temp_dir().join(DECOMPILED_DIR);
    let _ = fs::create_dir_all(&root);
    loop {
        let directory = root.join(format!(
            "session_{scope}_{}_{started_at}_{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&directory) {
            Ok(()) => return directory,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                sequence = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => return directory,
        }
    }
}

fn cache_path_in(directory: &Path, uri: &str) -> PathBuf {
    let digest = hex::encode(Sha1::digest(uri.as_bytes()));
    let raw_name = uri
        .rsplit_once("%28")
        .and_then(|(_, rest)| rest.strip_suffix(".class"))
        .or_else(|| {
            uri.split('?')
                .next()
                .and_then(|path| path.rsplit('/').next())
                .and_then(|segment| {
                    segment
                        .strip_suffix(".java")
                        .or(segment.strip_suffix(".class"))
                })
        })
        .unwrap_or("Decompiled");
    let mut name: String = raw_name
        .chars()
        .enumerate()
        .map(|(index, character)| {
            let valid = if index == 0 {
                character.is_ascii_alphabetic() || matches!(character, '_' | '$')
            } else {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
            };
            if valid {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name.push_str("Decompiled");
    }

    directory
        .join(format!("uri_{digest}"))
        .join(format!("{name}.java"))
}

fn sidecar_path(target: &Path) -> PathBuf {
    target.with_added_extension("jdt-uri")
}

fn ensure_jdt_uri_sidecar(target: &Path, jdt_uri: &str) {
    let sidecar = sidecar_path(target);
    if sidecar.is_file() {
        return;
    }
    let _ = fs::write(&sidecar, jdt_uri);
}

/// Resolve the original `jdt://` URI for a cached decompiled source file.
///
/// The mapping is stored on disk next to the source file so it survives across
/// proxy processes. Zed opens decompiled files in their own worktree, which
/// spawns a fresh `java-lsp-proxy`; that process must be able to reverse-map the
/// temporary `file://` document back to its `jdt://` class URI.
pub(crate) fn jdt_uri_for_cached_file(target: &Path) -> Option<String> {
    let content = fs::read_to_string(sidecar_path(target)).ok()?;
    let uri = content.trim();
    (!uri.is_empty()).then(|| uri.to_string())
}

fn write_cached_source(directory: &Path, uri: &str, content: &[u8]) -> Option<String> {
    if content.is_empty() {
        return None;
    }

    if let Err(error) = fs::create_dir_all(directory) {
        lsp_error!(
            "[decompile] Failed to create {}: {error}",
            directory.display()
        );
        return None;
    }
    let target = cache_path_in(directory, uri);
    if target.is_file() {
        ensure_jdt_uri_sidecar(&target, uri);
        return Some(path_to_file_uri(&target));
    }
    if let Some(parent) = target.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            lsp_error!("[decompile] Failed to create {}: {error}", parent.display());
            return None;
        }
    }

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = target.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        file.flush()?;
        fs::rename(&temporary, &target)
    })();

    match result {
        Ok(()) => {
            ensure_jdt_uri_sidecar(&target, uri);
            Some(path_to_file_uri(&target))
        }
        Err(_error) if target.is_file() => {
            let _ = fs::remove_file(&temporary);
            ensure_jdt_uri_sidecar(&target, uri);
            Some(path_to_file_uri(&target))
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            lsp_error!(
                "[decompile] Failed to atomically write {}: {error}",
                target.display()
            );
            None
        }
    }
}

fn collect_jdt_location_uris(value: &Value, uris: &mut Vec<String>, seen: &mut HashSet<String>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_jdt_location_uris(value, uris, seen);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "uri" | "targetUri") {
                    if let Value::String(uri) = value {
                        if uri.starts_with("jdt://") && seen.insert(uri.clone()) {
                            uris.push(uri.clone());
                        }
                    }
                    continue;
                }
                collect_jdt_location_uris(value, uris, seen);
            }
        }
        _ => {}
    }
}

fn replace_jdt_location_uris(value: &mut Value, replacements: &HashMap<String, String>) -> bool {
    match value {
        Value::Array(values) => {
            let mut rewritten = false;
            for value in values {
                rewritten |= replace_jdt_location_uris(value, replacements);
            }
            rewritten
        }
        Value::Object(object) => {
            let mut rewritten = false;
            for (key, value) in object {
                if matches!(key.as_str(), "uri" | "targetUri") {
                    if let Value::String(uri) = value {
                        if let Some(file_uri) = replacements.get(uri) {
                            *uri = file_uri.clone();
                            rewritten = true;
                        }
                    }
                    continue;
                }
                rewritten |= replace_jdt_location_uris(value, replacements);
            }
            rewritten
        }
        _ => false,
    }
}

fn jdt_uri_end(value: &str) -> usize {
    if let Some(class_end) = value.find(".class") {
        let class_end = class_end + ".class".len();
        if value[class_end..].starts_with('?') {
            return value[class_end..]
                .find(is_jdt_uri_terminator)
                .map(|query_end| class_end + query_end)
                .unwrap_or(value.len());
        }
        return class_end;
    }
    value.find(is_jdt_uri_terminator).unwrap_or(value.len())
}

fn is_jdt_uri_terminator(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            ')' | ']' | '}' | '"' | '>' | '`' | '\'' | ',' | ';'
        )
}

fn collect_jdt_uris(value: &Value, uris: &mut Vec<String>, seen: &mut HashSet<String>) {
    match value {
        Value::String(string) => {
            let mut rest = string.as_str();
            while let Some(position) = rest.find("jdt://") {
                let tail = &rest[position..];
                let end = jdt_uri_end(tail);
                let uri = tail[..end].to_string();
                if seen.insert(uri.clone()) {
                    uris.push(uri);
                }
                rest = &tail[end..];
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_jdt_uris(value, uris, seen);
            }
        }
        Value::Object(object) => {
            for value in object.values() {
                collect_jdt_uris(value, uris, seen);
            }
        }
        _ => {}
    }
}

fn replace_in_strings(value: &mut Value, replacements: &HashMap<String, String>) {
    match value {
        Value::String(string) => {
            for (from, to) in replacements {
                if string.contains(from) {
                    *string = string.replace(from, to);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                replace_in_strings(value, replacements);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                replace_in_strings(value, replacements);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    struct FakeFetcher {
        calls: AtomicUsize,
        cancellations: AtomicUsize,
        active: AtomicUsize,
        max_active: AtomicUsize,
        delay: Duration,
        succeeds: bool,
    }

    impl FakeFetcher {
        fn new(delay: Duration) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                cancellations: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                delay,
                succeeds: true,
            }
        }

        fn failing(delay: Duration) -> Self {
            Self {
                succeeds: false,
                ..Self::new(delay)
            }
        }
    }

    impl ClassContentFetcher for FakeFetcher {
        fn fetch(&self, uri: &str, _request_id: Value) -> Option<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            thread::sleep(self.delay);
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.succeeds.then(|| format!("class {} {{}}", uri.len()))
        }

        fn cancel(&self, _request_id: &Value) {
            self.cancellations.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn unique_uri(name: &str) -> String {
        format!(
            "jdt://contents/test/{name}-{}.class",
            AtomicU64::new(1).fetch_add(1, Ordering::Relaxed) + std::process::id() as u64
        )
    }

    fn rewrite_for_test(
        coordinator: &DecompileCoordinator,
        token: u64,
        uris: &[String],
        priority: Priority,
    ) -> HashMap<String, String> {
        resolve_uris(
            &coordinator.inner,
            token,
            uris,
            priority,
            Instant::now() + Duration::from_secs(2),
        )
    }

    fn cached_path(coordinator: &DecompileCoordinator, uri: &str) -> PathBuf {
        cache_path_in(&coordinator.inner.cache_dir, uri)
    }

    #[test]
    fn rewrites_nested_workspace_symbol_locations() {
        let mut result = json!([
            {
                "location": {
                    "uri": "jdt://contents/java.base/java.lang/String.class"
                }
            },
            {
                "location": {
                    "uri": "file:///workspace/ProjectString.java"
                }
            }
        ]);
        let replacements = HashMap::from([(
            "jdt://contents/java.base/java.lang/String.class".to_string(),
            "file:///tmp/String.java".to_string(),
        )]);

        assert!(replace_jdt_location_uris(&mut result, &replacements));
        assert_eq!(
            result[0]["location"]["uri"],
            json!("file:///tmp/String.java")
        );
        assert_eq!(
            result[1]["location"]["uri"],
            json!("file:///workspace/ProjectString.java")
        );
    }

    #[test]
    fn deduplicates_location_uris_before_resolution() {
        let uri = "jdt://contents/java.base/java.lang/String.class";
        let result = json!([{ "uri": uri }, { "targetUri": uri }]);
        let mut uris = Vec::new();

        collect_jdt_location_uris(&result, &mut uris, &mut HashSet::new());

        assert_eq!(uris, vec![uri]);
    }

    #[test]
    fn ignores_jdt_uris_outside_location_fields() {
        let result = json!({
            "documentation": "jdt://contents/java.base/java.lang/String.class",
            "data": {
                "sourceUri": "jdt://contents/java.base/java.lang/Object.class"
            }
        });
        let mut uris = Vec::new();

        collect_jdt_location_uris(&result, &mut uris, &mut HashSet::new());

        assert!(uris.is_empty());
    }

    #[test]
    fn embedded_uri_extraction_stops_before_sentence_punctuation() {
        let uri = "jdt://contents/java.base/java.lang/String.class";
        let result = json!(format!("See {uri}, then java.lang.Object;"));
        let mut uris = Vec::new();

        collect_jdt_uris(&result, &mut uris, &mut HashSet::new());

        assert_eq!(uris, vec![uri]);
    }

    #[test]
    fn embedded_uri_extraction_preserves_query_components() {
        let uri = "jdt://contents/java.base/java.lang/String.class?=java.base/String";
        let result = json!(format!("See {uri}, then continue."));
        let mut uris = Vec::new();

        collect_jdt_uris(&result, &mut uris, &mut HashSet::new());

        assert_eq!(uris, vec![uri]);
    }

    #[test]
    fn concurrent_waiters_share_one_fetch() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::from_millis(20)));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher.clone(), "test-".to_string(), 2, 0);
        let uri = unique_uri("single-flight");
        let _ = fs::remove_file(cached_path(&coordinator, &uri));

        let first = {
            let coordinator = coordinator.clone();
            let uri = uri.clone();
            thread::spawn(move || rewrite_for_test(&coordinator, 1, &[uri], Priority::Interactive))
        };
        let second = {
            let coordinator = coordinator.clone();
            let uri = uri.clone();
            thread::spawn(move || rewrite_for_test(&coordinator, 2, &[uri], Priority::Interactive))
        };

        assert_eq!(first.join().unwrap().len(), 1);
        assert_eq!(second.join().unwrap().len(), 1);
        assert_eq!(fetcher.calls.load(Ordering::Relaxed), 1);
        let _ = fs::remove_file(cached_path(&coordinator, &uri));
        coordinator.shutdown();
    }

    #[test]
    fn interactive_fetches_use_bounded_parallelism() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::from_millis(30)));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher.clone(), "test-".to_string(), 2, 0);
        let uris: Vec<_> = (0..4)
            .map(|index| unique_uri(&format!("parallel-{index}")))
            .collect();
        for uri in &uris {
            let _ = fs::remove_file(cached_path(&coordinator, uri));
        }

        assert_eq!(
            rewrite_for_test(&coordinator, 1, &uris, Priority::Interactive).len(),
            uris.len()
        );
        assert_eq!(fetcher.max_active.load(Ordering::SeqCst), 2);
        for uri in &uris {
            let _ = fs::remove_file(cached_path(&coordinator, uri));
        }
        coordinator.shutdown();
    }

    #[test]
    fn deadline_returns_completed_replacements_only() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::from_millis(50)));
        let coordinator = DecompileCoordinator::with_fetcher(fetcher, "test-".to_string(), 1, 0);
        let uris = vec![unique_uri("deadline-a"), unique_uri("deadline-b")];
        for uri in &uris {
            let _ = fs::remove_file(cached_path(&coordinator, uri));
        }

        let replacements = resolve_uris(
            &coordinator.inner,
            1,
            &uris,
            Priority::Interactive,
            Instant::now() + Duration::from_millis(70),
        );

        assert_eq!(replacements.len(), 1);
        for uri in &uris {
            let _ = fs::remove_file(cached_path(&coordinator, uri));
        }
        coordinator.shutdown();
    }

    #[test]
    fn queued_jobs_complete_when_their_deadline_expires() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::from_millis(150)));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher.clone(), "deadline-".to_string(), 1, 1);
        let first_uri = unique_uri("blocking-job");
        let second_uri = unique_uri("queued-deadline");
        let (first_tx, _first_rx) = mpsc::channel();
        assert!(coordinator
            .submit(RewriteJob {
                token: 1,
                message: json!({ "result": [{ "uri": first_uri }] }),
                mode: RewriteMode::Locations,
                priority: Priority::Interactive,
                deadline: Instant::now() + Duration::from_secs(1),
                complete: Box::new(move |message| {
                    let _ = first_tx.send(message);
                }),
            })
            .is_ok());
        let worker_start_deadline = Instant::now() + Duration::from_secs(1);
        while fetcher.active.load(Ordering::SeqCst) == 0 && Instant::now() < worker_start_deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(fetcher.active.load(Ordering::SeqCst), 1);

        let original = json!({ "result": [{ "uri": second_uri }] });
        let (second_tx, second_rx) = mpsc::channel();
        assert!(coordinator
            .submit(RewriteJob {
                token: 2,
                message: original.clone(),
                mode: RewriteMode::Locations,
                priority: Priority::Interactive,
                deadline: Instant::now() + Duration::from_millis(20),
                complete: Box::new(move |message| {
                    let _ = second_tx.send(message);
                }),
            })
            .is_ok());

        assert_eq!(
            second_rx.recv_timeout(Duration::from_millis(500)).unwrap(),
            original
        );
        coordinator.shutdown();
    }

    #[test]
    fn canceling_one_waiter_keeps_shared_fetch_alive() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::from_millis(50)));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher.clone(), "test-".to_string(), 1, 0);
        let uri = unique_uri("shared-cancel");
        let _ = fs::remove_file(cached_path(&coordinator, &uri));

        let first = {
            let coordinator = coordinator.clone();
            let uri = uri.clone();
            thread::spawn(move || rewrite_for_test(&coordinator, 1, &[uri], Priority::Interactive))
        };
        let second = {
            let coordinator = coordinator.clone();
            let uri = uri.clone();
            thread::spawn(move || rewrite_for_test(&coordinator, 2, &[uri], Priority::Interactive))
        };
        while {
            let state = coordinator.inner.state.lock().unwrap();
            state
                .uris
                .get(&uri)
                .is_none_or(|entry| entry.waiters.len() < 2)
        } {
            thread::yield_now();
        }
        coordinator.cancel(1);

        assert!(first.join().unwrap().is_empty());
        assert_eq!(second.join().unwrap().len(), 1);
        assert_eq!(fetcher.cancellations.load(Ordering::Relaxed), 0);
        let _ = fs::remove_file(cached_path(&coordinator, &uri));
        coordinator.shutdown();
    }

    #[test]
    fn timed_out_fetch_does_not_poison_immediate_retry() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::from_millis(60)));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher.clone(), "retry-".to_string(), 2, 0);
        let uri = unique_uri("retry-after-timeout");

        let first = resolve_uris(
            &coordinator.inner,
            1,
            std::slice::from_ref(&uri),
            Priority::Interactive,
            Instant::now() + Duration::from_millis(10),
        );
        let second = resolve_uris(
            &coordinator.inner,
            2,
            std::slice::from_ref(&uri),
            Priority::Interactive,
            Instant::now() + Duration::from_millis(250),
        );

        assert!(first.is_empty());
        assert_eq!(second.len(), 1);
        assert_eq!(fetcher.calls.load(Ordering::Relaxed), 2);
        assert_eq!(fetcher.cancellations.load(Ordering::Relaxed), 1);
        coordinator.shutdown();
    }

    #[test]
    fn negative_cache_prevents_immediate_retry_storms() {
        let fetcher = Arc::new(FakeFetcher::failing(Duration::ZERO));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher.clone(), "test-".to_string(), 1, 0);
        let uri = unique_uri("negative-cache");
        let _ = fs::remove_file(cached_path(&coordinator, &uri));

        assert!(rewrite_for_test(
            &coordinator,
            1,
            std::slice::from_ref(&uri),
            Priority::Interactive
        )
        .is_empty());
        assert!(rewrite_for_test(
            &coordinator,
            2,
            std::slice::from_ref(&uri),
            Priority::Interactive
        )
        .is_empty());
        assert_eq!(fetcher.calls.load(Ordering::Relaxed), 1);
        coordinator.shutdown();
    }

    #[test]
    fn coordinators_do_not_share_cache_across_server_sessions() {
        let uri = unique_uri("session-cache");
        let first_fetcher = Arc::new(FakeFetcher::new(Duration::ZERO));
        let first = DecompileCoordinator::with_fetcher(
            first_fetcher.clone(),
            "same-server-scope-".to_string(),
            1,
            0,
        );
        assert_eq!(
            rewrite_for_test(&first, 1, std::slice::from_ref(&uri), Priority::Interactive).len(),
            1
        );
        let first_cache = first.inner.cache_dir.clone();
        first.shutdown();
        first.cleanup_cache();
        assert!(!first_cache.exists());

        let second_fetcher = Arc::new(FakeFetcher::new(Duration::ZERO));
        let second = DecompileCoordinator::with_fetcher(
            second_fetcher.clone(),
            "same-server-scope-".to_string(),
            1,
            0,
        );
        assert_ne!(first_cache, second.inner.cache_dir);
        assert_eq!(
            rewrite_for_test(
                &second,
                2,
                std::slice::from_ref(&uri),
                Priority::Interactive
            )
            .len(),
            1
        );
        assert_eq!(second_fetcher.calls.load(Ordering::Relaxed), 1);
        second.shutdown();
        second.cleanup_cache();
    }

    #[test]
    fn canceling_a_queued_bulk_job_retires_all_state() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::ZERO));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher, "queued-cancel-".to_string(), 0, 0);
        assert!(coordinator
            .submit(RewriteJob {
                token: 7,
                message: json!({ "result": [] }),
                mode: RewriteMode::Locations,
                priority: Priority::Bulk,
                deadline: Instant::now() + Duration::from_secs(1),
                complete: Box::new(|_| {}),
            })
            .is_ok());

        coordinator.cancel(7);

        let state = coordinator.inner.state.lock().unwrap();
        assert!(!state.canceled_jobs.contains(&7));
        assert!(state.bulk_jobs.is_empty());
        drop(state);
        coordinator.shutdown();
    }

    #[test]
    fn shutdown_completes_queued_jobs_unchanged() {
        let fetcher = Arc::new(FakeFetcher::new(Duration::ZERO));
        let coordinator =
            DecompileCoordinator::with_fetcher(fetcher, "shutdown-".to_string(), 0, 0);
        let original = json!({ "result": [{ "uri": "jdt://contents/test/A.class" }] });
        let (sender, receiver) = mpsc::channel();
        assert!(coordinator
            .submit(RewriteJob {
                token: 8,
                message: original.clone(),
                mode: RewriteMode::Locations,
                priority: Priority::Interactive,
                deadline: Instant::now() + Duration::from_secs(1),
                complete: Box::new(move |message| {
                    let _ = sender.send(message);
                }),
            })
            .is_ok());

        coordinator.shutdown();

        assert_eq!(receiver.recv().unwrap(), original);
    }

    #[test]
    fn competing_cache_writes_never_expose_partial_content() {
        let uri = unique_uri("atomic-cache");
        let directory = session_cache_dir("atomic-cache-test");
        let path = cache_path_in(&directory, &uri);
        let _ = fs::remove_file(&path);
        let first_content = vec![b'a'; 32 * 1024];
        let second_content = vec![b'b'; 32 * 1024];

        let first = {
            let uri = uri.clone();
            let content = first_content.clone();
            let directory = directory.clone();
            thread::spawn(move || write_cached_source(&directory, &uri, &content))
        };
        let second = {
            let uri = uri.clone();
            let content = second_content.clone();
            let directory = directory.clone();
            thread::spawn(move || write_cached_source(&directory, &uri, &content))
        };
        assert!(first.join().unwrap().is_some());
        assert!(second.join().unwrap().is_some());

        let cached = fs::read(&path).unwrap();
        assert!(cached == first_content || cached == second_content);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cache_names_use_stable_digest_and_sanitized_name() {
        let directory = Path::new("/tmp/cache");
        let first = cache_path_in(directory, "jdt://contents/a/../../Bad Name.class");
        let second = cache_path_in(directory, "jdt://contents/a/../../Bad Name.class");

        assert_eq!(first, second);
        assert_eq!(first.file_name().unwrap(), "Bad_Name.java");
        assert_eq!(first.extension().unwrap(), "java");
        assert!(first
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("uri_"));
    }

    #[test]
    fn cache_file_stems_are_valid_java_identifiers() {
        let directory = Path::new("/tmp/cache");
        let path = cache_path_in(directory, "jdt://contents/a/Products$Product$Request.class");
        let stem = path.file_stem().unwrap().to_string_lossy();

        assert_eq!(stem, "Products$Product$Request");
        assert!(stem
            .chars()
            .enumerate()
            .all(|(index, character)| if index == 0 {
                character.is_ascii_alphabetic() || matches!(character, '_' | '$')
            } else {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
            }));
    }

    #[test]
    fn empty_sources_are_not_cached() {
        let uri = unique_uri("empty");
        let directory = session_cache_dir("empty-cache-test");
        let path = cache_path_in(&directory, &uri);
        let _ = fs::remove_file(&path);

        assert_eq!(write_cached_source(&directory, &uri, b""), None);
        assert!(!path.exists());
    }

    #[test]
    fn cached_sources_keep_reverse_mapping() {
        let uri = "jdt://contents/java.base/java.lang/String.class";
        let directory = session_cache_dir("reverse-mapping-test");
        let path = cache_path_in(&directory, uri);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(sidecar_path(&path));

        let file_uri = write_cached_source(&directory, uri, b"public class String {}").unwrap();
        assert_eq!(file_uri, path_to_file_uri(&path));
        assert_eq!(jdt_uri_for_cached_file(&path).as_deref(), Some(uri));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(sidecar_path(&path));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn reverse_mapping_is_missing_for_unrelated_files() {
        let path = PathBuf::from("/tmp/no-sidecar/String.java");
        assert_eq!(jdt_uri_for_cached_file(&path), None);
    }
}
