//! A small, blocking client for the Enki daemon (`sharedai-daemon`): list its models and stream a
//! chat reply. Qanloom does not link llama.cpp; the daemon's worker does.
//!
//! SharedAI runs models on the user's own machine. Its wire contract (docs/INFERENCE.md there) is
//! newline-delimited JSON over a private Unix socket: negotiate a protocol version, send one
//! request, read frames until exactly one terminal frame (`done`, `busy`, `inference_error`).
//!
//! This crate speaks that contract with `std` only (no async runtime), so the editor can run a
//! request on a plain thread. It is a clean implementation of the published contract, not a copy
//! of the SharedAI SDK. On targets without Unix sockets (Windows, wasm) every call returns
//! [`Error::Unsupported`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// GGUF file checks. Chat runs through [`chat_file`].
pub mod local;
/// Pinned assistant models and verified downloads.
pub mod models;

mod selection;
pub use selection::select_text_model;

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

/// Protocol version for health and the model registry.
pub const VERSION_BASE: u64 = 1;
/// Protocol version that adds inference.
pub const VERSION_INFERENCE: u64 = 2;
/// Version 1 frames are at most 64 KiB; version 2 frames at most 4 MiB (both without the newline).
pub const MAX_FRAME_V1: usize = 64 << 10;
pub const MAX_FRAME_V2: usize = 4 << 20;
/// The daemon's caps: at most this many messages and `max_tokens`.
pub const MAX_MESSAGES: usize = 256;
pub const MAX_TOKENS: u32 = 4096;

/// How long to wait for the first event (it may include loading the model) and for each later one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub first: Duration,
    pub next: Duration,
}
impl Default for Timeouts {
    fn default() -> Self {
        Self { first: Duration::from_secs(360), next: Duration::from_secs(60) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// "system", "user" or "assistant".
    pub role: String,
    pub content: String,
}
impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into() }
    }
}

/// Sampling controls; `None` takes the daemon's default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub seed: Option<u32>,
}

/// How a finished stream ended, as the daemon reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// `stop`, `length`, `stop_sequence`, `cancelled` or `time_limit`.
    pub reason: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

#[derive(Debug)]
pub enum Error {
    /// No socket path: neither the setting, `SHAREDAI_SOCKET` nor `$XDG_RUNTIME_DIR` gave one.
    NoEndpoint,
    /// This platform has no Unix sockets.
    Unsupported,
    Io(std::io::Error),
    /// A frame that isn't valid JSON, is too large, or isn't the expected reply.
    Protocol(String),
    /// The daemon predates inference.
    Incompatible,
    Busy {
        retry_after: Duration,
    },
    /// `kind` is the daemon's error kind (`model_not_found`, `context_overflow`, …).
    Inference {
        kind: String,
        message: String,
    },
    Timeout,
    Cancelled,
    /// A request outside the daemon's limits, refused before anything was sent.
    Invalid(String),
    /// No `sharedai-daemon` next to the app, in `SHAREDAI_DAEMON` or on `PATH`.
    NotInstalled,
    /// The daemon was found but didn't start listening.
    StartFailed(String),
    /// The model registry refused a registration (`kind`: `invalid_metadata`, `capacity`, …).
    Registry(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEndpoint => write!(f, "no model service socket (SHAREDAI_SOCKET or XDG_RUNTIME_DIR)"),
            Self::Unsupported => write!(f, "SharedAI isn't available on this platform"),
            Self::Io(e) if e.kind() == std::io::ErrorKind::NotFound || e.kind() == std::io::ErrorKind::ConnectionRefused => {
                write!(f, "SharedAI isn't running ({e})")
            }
            Self::Io(e) => write!(f, "SharedAI connection: {e}"),
            Self::Protocol(m) => write!(f, "SharedAI sent an unexpected reply: {m}"),
            Self::Incompatible => write!(f, "This SharedAI daemon has no local inference; update it"),
            Self::Busy { retry_after } => write!(f, "SharedAI is busy; try again in {} s", retry_after.as_secs().max(1)),
            Self::Inference { kind, message } => write!(f, "SharedAI ({kind}): {message}"),
            Self::Timeout => write!(f, "SharedAI stopped responding"),
            Self::Cancelled => write!(f, "Stopped"),
            Self::Invalid(m) => write!(f, "{m}"),
            Self::NotInstalled => write!(f, "sharedai-daemon isn't installed"),
            Self::StartFailed(m) => write!(f, "sharedai-daemon didn't start: {m}"),
            Self::Registry(k) => write!(f, "the model registry refused the model ({k})"),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// The socket to use: `configured` if non-empty, else `$SHAREDAI_SOCKET`, else
/// `$XDG_RUNTIME_DIR/sharedai/ipc.sock` (the Qanloom desktop's convention).
pub fn endpoint(configured: &str) -> Result<PathBuf, Error> {
    let configured = configured.trim();
    if !configured.is_empty() {
        return Ok(PathBuf::from(configured));
    }
    if let Some(p) = std::env::var_os("SHAREDAI_SOCKET").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|p| !p.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir).join("sharedai").join("ipc.sock")),
        None => Err(Error::NoEndpoint),
    }
}

/// A newline-terminated JSON frame for `request`, checked against `limit`.
pub fn encode(request: &Value, limit: usize) -> Result<Vec<u8>, Error> {
    let mut data = serde_json::to_vec(request).map_err(|e| Error::Protocol(e.to_string()))?;
    if data.len() > limit {
        return Err(Error::Invalid(format!("The request is larger than SharedAI accepts ({} KiB)", limit >> 10)));
    }
    data.push(b'\n');
    Ok(data)
}

/// The chat request frame (protocol v2), after checking the daemon's limits.
pub fn chat_request(model: &str, messages: &[Message], params: &Params) -> Result<Value, Error> {
    if model.trim().is_empty() {
        return Err(Error::Invalid("Choose a SharedAI model first".into()));
    }
    if messages.is_empty() || messages.len() > MAX_MESSAGES {
        return Err(Error::Invalid(format!("A chat has 1 to {MAX_MESSAGES} messages")));
    }
    let mut p = serde_json::Map::new();
    if let Some(n) = params.max_tokens {
        p.insert("max_tokens".into(), json!(n.clamp(1, MAX_TOKENS)));
    }
    if let Some(t) = params.temperature.filter(|t| t.is_finite()) {
        p.insert("temperature".into(), json!(t.clamp(0.0, 2.0)));
    }
    if let Some(s) = params.seed {
        p.insert("seed".into(), json!(s));
    }
    let messages: Vec<Value> = messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect();
    Ok(json!({"type": "chat", "model": model, "messages": messages, "params": Value::Object(p)}))
}

/// One parsed stream event.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Chunk { seq: u64, text: String },
    Done(Summary),
}

/// Interpret one frame of a v2 stream. Terminal errors come back as `Err`.
pub fn parse_event(frame: &[u8]) -> Result<Event, Error> {
    let v: Value = serde_json::from_slice(frame).map_err(|e| Error::Protocol(e.to_string()))?;
    let str_of = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let num_of = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    match v.get("type").and_then(Value::as_str) {
        Some("chunk") => Ok(Event::Chunk { seq: num_of("seq"), text: str_of("text") }),
        Some("done") => Ok(Event::Done(Summary {
            reason: str_of("reason"),
            prompt_tokens: num_of("prompt_tokens"),
            completion_tokens: num_of("completion_tokens"),
        })),
        Some("busy") => Err(Error::Busy { retry_after: Duration::from_millis(num_of("retry_after_ms")) }),
        Some("inference_error") => Err(Error::Inference { kind: str_of("kind"), message: str_of("message") }),
        Some("incompatible") => Err(Error::Incompatible),
        Some("error") => Err(Error::Protocol(str_of("message"))),
        other => Err(Error::Protocol(format!("frame type {other:?}"))),
    }
}

/// Reads bounded newline-delimited frames, keeping partial input across read timeouts.
pub struct FrameReader<R> {
    inner: R,
    buf: Vec<u8>,
    limit: usize,
}
impl<R: std::io::Read> FrameReader<R> {
    pub fn new(inner: R, limit: usize) -> Self {
        Self { inner, buf: Vec::new(), limit }
    }

    /// The next frame; `Ok(None)` at a clean end of stream. A read timeout surfaces as the
    /// `WouldBlock`/`TimedOut` I/O error with buffered bytes kept for the next call.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, Error> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                let rest = self.buf.split_off(i + 1);
                let mut frame = std::mem::replace(&mut self.buf, rest);
                frame.pop();
                return Ok(Some(frame));
            }
            if self.buf.len() > self.limit {
                return Err(Error::Protocol("frame too large".into()));
            }
            let mut chunk = [0u8; 8192];
            let n = self.inner.read(&mut chunk)?;
            if n == 0 {
                return if self.buf.is_empty() { Ok(None) } else { Err(Error::Protocol("truncated frame".into())) };
            }
            self.buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
        }
    }
}

fn is_timeout(e: &Error) -> bool {
    matches!(e, Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// Drive one stream to its terminal frame: `on_chunk` gets each piece of text in order. Polls
/// `cancel` between reads; on cancel it sends `cancel` and returns [`Error::Cancelled`].
pub fn drive<S: std::io::Read>(
    reader: &mut FrameReader<S>,
    cancel: &AtomicBool,
    timeouts: Timeouts,
    mut write_cancel: impl FnMut(&[u8]) -> std::io::Result<()>,
    mut on_chunk: impl FnMut(&str),
) -> Result<Summary, Error> {
    let mut expect_seq = 0u64;
    let mut waited = Duration::ZERO;
    let mut limit = timeouts.first;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = write_cancel(b"{\"type\":\"cancel\"}\n");
            return Err(Error::Cancelled);
        }
        match reader.next_frame() {
            Ok(Some(frame)) => {
                waited = Duration::ZERO;
                limit = timeouts.next;
                match parse_event(&frame)? {
                    Event::Chunk { seq, text } => {
                        if seq != expect_seq {
                            return Err(Error::Protocol(format!("chunk {seq} out of order, expected {expect_seq}")));
                        }
                        expect_seq = expect_seq.saturating_add(1);
                        on_chunk(&text);
                    }
                    Event::Done(summary) => return Ok(summary),
                }
            }
            Ok(None) => return Err(Error::Protocol("the stream ended without a result".into())),
            Err(e) if is_timeout(&e) => {
                waited = waited.saturating_add(POLL);
                if waited >= limit {
                    return Err(Error::Timeout);
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// How often a waiting stream checks for cancellation.
pub const POLL: Duration = Duration::from_millis(200);

/// A registry id for a model file: its name without the extension, reduced to the characters
/// the registry allows (letters, digits, `.`, `_`, `-`; at most 128).
pub fn model_id(path: &std::path::Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let id: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).take(128).collect();
    let id = id.trim_matches(|c| c == '-' || c == '.').to_string();
    if id.is_empty() { "model".into() } else { id }
}

/// Chat with the GGUF file at `path` through the Enki daemon.
///
/// Starts `sharedai-daemon` when nothing is listening, registers the file, and streams the reply.
/// A catalog model is refused when its bytes do not match the pinned checksum. The file path is
/// what Scribe stores; the daemon is asked for that file's registry id.
pub fn chat_file(path: &str, messages: &[Message], params: &Params, cancel: &AtomicBool, on_chunk: impl FnMut(&str)) -> Result<Summary, Error> {
    if messages.len() > MAX_MESSAGES
        || messages.iter().try_fold(0usize, |sum, m| sum.checked_add(m.content.len())?.checked_add(m.role.len())).is_none_or(|n| n > 128 * 1024)
    {
        return Err(Error::Invalid("Message too large".into()));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let file = std::path::Path::new(path);
    local::validate_model(file)?;
    let file = file.canonicalize()?;
    let (size, sha256) = hash_file(&file)?;
    let license = if let Some(spec) = models::CATALOG.iter().find(|m| file.file_name().is_some_and(|name| name == m.file)) {
        if size != spec.size || sha256 != spec.sha256 {
            return Err(Error::Invalid("The model checksum does not match. Download it again.".into()));
        }
        spec.license
    } else {
        "unknown"
    };
    #[cfg(not(unix))]
    {
        let _ = (params, on_chunk, size, sha256, license);
        Err(Error::Unsupported)
    }
    #[cfg(unix)]
    {
        let endpoint = endpoint("")?;
        ensure_running(&endpoint, Some(&endpoint))?;
        let size = i64::try_from(size).map_err(|_| Error::Invalid("the model file is too large".into()))?;
        let id = model_id(&file);
        net::register_known(&endpoint, "nisaba", &id, &file, size, &sha256, license)?;
        chat(&endpoint, "nisaba", &id, messages, params, Timeouts::default(), cancel, on_chunk)
    }
}

/// Size and lowercase hex SHA-256 of a file, read in 1 MiB pieces.
pub fn hash_file(path: &std::path::Path) -> Result<(u64, String), Error> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(buf.get(..n).unwrap_or(&[]));
        size = size.saturating_add(n as u64);
    }
    let hex = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok((size, hex))
}

/// Where to find `sharedai-daemon`: `SHAREDAI_DAEMON`, then `SHAREDAI_BIN_DIR`, then the newest
/// shared install (`/usr/lib/enki` or `$XDG_DATA_HOME/enki`), then the program's private helper
/// folder, `<prefix>/lib/<program>/`, then next to the program, then each `PATH` directory.
pub fn find_daemon() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("SHAREDAI_DAEMON").filter(|p| !p.is_empty()) {
        return Some(p.into()).filter(|p: &std::path::PathBuf| p.is_file());
    }
    if let Some(dir) = std::env::var_os("SHAREDAI_BIN_DIR").filter(|p| !p.is_empty()) {
        let bundled = std::path::PathBuf::from(dir).join("sharedai-daemon");
        if bundled.is_file() {
            return Some(bundled);
        }
    }
    let exe = std::env::current_exe().ok();
    let mut candidates = Vec::new();
    if let Some(shared) = newest_daemon(&installed_daemon_paths()) {
        candidates.push(shared);
    }
    candidates.extend(daemon_candidates(exe.as_deref(), std::env::var_os("PATH").as_deref()));
    candidates.into_iter().find(|p| p.is_file())
}

/// `/usr/lib/enki` first, then the per-user install. [`newest_daemon`] keeps the first on a tie.
fn installed_daemon_paths() -> Vec<std::path::PathBuf> {
    let mut paths = vec![std::path::PathBuf::from("/usr/lib/enki/sharedai-daemon")];
    if let Some(data) = data_home() {
        paths.push(data.join("enki").join("sharedai-daemon"));
    }
    paths
}

fn data_home() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return Some(std::path::PathBuf::from(dir));
    }
    std::env::var_os("HOME").filter(|home| !home.is_empty()).map(|home| std::path::PathBuf::from(home).join(".local/share"))
}

/// The newest `sharedai-daemon` in `paths`, compared by the `VERSION` file in its directory.
/// A missing or unreadable `VERSION` sorts as `0.0.0`. The earlier path wins when versions match.
fn newest_daemon(paths: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    let mut best: Option<(std::path::PathBuf, (u64, u64, u64, u64))> = None;
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let dir = path.parent().unwrap_or(path.as_path());
        let key = version_key(dir);
        let replace = match &best {
            None => true,
            Some((_, old)) => key > *old,
        };
        if replace {
            best = Some((path.clone(), key));
        }
    }
    best.map(|(path, _)| path)
}

/// `VERSION` is `major.minor.patch stamp`. Non-numeric text sorts as zero.
fn version_key(dir: &std::path::Path) -> (u64, u64, u64, u64) {
    let Ok(text) = std::fs::read_to_string(dir.join("VERSION")) else {
        return (0, 0, 0, 0);
    };
    let mut words = text.split_whitespace();
    let version = words.next().unwrap_or("0");
    let stamp = words.next().and_then(|word| word.parse().ok()).unwrap_or(0);
    let mut nums = [0u64; 3];
    for (index, part) in version.split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        nums[index] = digits.parse().unwrap_or(0);
    }
    (nums[0], nums[1], nums[2], stamp)
}

/// The places [`find_daemon`] looks, in order, for a program at `exe` and a `PATH` value.
pub fn daemon_candidates(exe: Option<&std::path::Path>, path: Option<&std::ffi::OsStr>) -> Vec<std::path::PathBuf> {
    let name = "sharedai-daemon";
    let mut out = Vec::new();
    if let Some(exe) = exe
        && let Some(dir) = exe.parent()
    {
        if let (Some(prefix), Some(stem)) = (dir.parent(), exe.file_stem()) {
            out.push(prefix.join("lib").join(stem).join(name));
        }
        out.push(dir.join(name));
    }
    if let Some(path) = path {
        out.extend(std::env::split_paths(path).map(|d| d.join(name)));
    }
    out
}

#[cfg(unix)]
mod net {
    use super::{
        AtomicBool, Duration, Error, FrameReader, MAX_FRAME_V1, MAX_FRAME_V2, Message, POLL, Params, Summary, Timeouts, VERSION_BASE,
        VERSION_INFERENCE, Value, chat_request, drive, encode, find_daemon, hash_file, json,
    };
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    fn connect(endpoint: &Path, app: &str, version: u64) -> Result<FrameReader<UnixStream>, Error> {
        let mut stream = UnixStream::connect(endpoint)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        stream.write_all(&encode(&json!({"type": "negotiate", "versions": [version], "app": app}), MAX_FRAME_V1)?)?;
        let limit = if version >= VERSION_INFERENCE { MAX_FRAME_V2 } else { MAX_FRAME_V1 };
        let mut reader = FrameReader::new(stream, limit);
        let frame = reader.next_frame()?.ok_or_else(|| Error::Protocol("no reply to negotiate".into()))?;
        let v: Value = serde_json::from_slice(&frame).map_err(|e| Error::Protocol(e.to_string()))?;
        match (v.get("type").and_then(Value::as_str), v.get("version").and_then(Value::as_u64)) {
            (Some("negotiated"), Some(got)) if got == version => Ok(reader),
            (Some("negotiated" | "incompatible"), _) => Err(Error::Incompatible),
            // Daemons from before negotiation answer "Invalid request"; they have no inference.
            (Some("error"), _) if version >= VERSION_INFERENCE => Err(Error::Incompatible),
            _ => Err(Error::Protocol(format!("negotiate reply {v}"))),
        }
    }

    /// Start the daemon unless something already listens on `endpoint`. `socket_env` is passed as
    /// `SHAREDAI_SOCKET` when set (otherwise the daemon uses its default, the same path).
    pub fn ensure_running(endpoint: &Path, socket_env: Option<&Path>) -> Result<(), Error> {
        use std::os::unix::process::CommandExt;
        if UnixStream::connect(endpoint).is_ok() {
            return Ok(());
        }
        let daemon = find_daemon().ok_or(Error::NotInstalled)?;
        if let Some(dir) = socket_env.and_then(Path::parent) {
            use std::os::unix::fs::DirBuilderExt;
            match std::fs::DirBuilder::new().mode(0o700).recursive(true).create(dir) {
                Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(Error::Io(e)),
                _ => {}
            }
        }
        let mut cmd = std::process::Command::new(&daemon);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        // Its own process group: it outlives this app and isn't stopped by the terminal's Ctrl+C.
        cmd.process_group(0);
        if let Some(sock) = socket_env {
            cmd.env("SHAREDAI_SOCKET", sock);
        }
        let mut child = cmd.spawn().map_err(|e| Error::StartFailed(e.to_string()))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if UnixStream::connect(endpoint).is_ok() {
                return Ok(());
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(Error::StartFailed(format!("exited with {status}")));
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::StartFailed("not listening after 10 s".into()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Register a model file: its checksum is computed here, the daemon checks it before loading.
    pub fn register_file(endpoint: &Path, app: &str, id: &str, path: &Path) -> Result<(), Error> {
        let (size, sha256) = hash_file(path)?;
        let size = i64::try_from(size).map_err(|_| Error::Invalid("the model file is too large".into()))?;
        register_known(endpoint, app, id, path, size, &sha256, "unknown")
    }

    pub(crate) fn register_known(endpoint: &Path, app: &str, id: &str, path: &Path, size: i64, sha256: &str, license: &str) -> Result<(), Error> {
        let model = json!({
            "id": id,
            "path": path.to_string_lossy(),
            "sha256": sha256,
            "size_bytes": size,
            "source": "local-file",
            "license": license,
        });
        let mut reader = connect(endpoint, app, VERSION_BASE)?;
        reader.inner.write_all(&encode(&json!({"type": "register_models", "models": [model]}), MAX_FRAME_V1)?)?;
        let frame = reader.next_frame()?.ok_or_else(|| Error::Protocol("no registration reply".into()))?;
        let v: Value = serde_json::from_slice(&frame).map_err(|e| Error::Protocol(e.to_string()))?;
        match v.get("type").and_then(Value::as_str) {
            Some("registered") => Ok(()),
            Some("registry_error") => Err(Error::Registry(v.get("kind").and_then(Value::as_str).unwrap_or("unknown").into())),
            _ => Err(Error::Protocol(format!("register reply {v}"))),
        }
    }

    pub fn list_models(endpoint: &Path, app: &str) -> Result<Vec<String>, Error> {
        let mut reader = connect(endpoint, app, VERSION_BASE)?;
        reader.inner.write_all(&encode(&json!({"type": "list_models"}), MAX_FRAME_V1)?)?;
        let frame = reader.next_frame()?.ok_or_else(|| Error::Protocol("no model list".into()))?;
        let v: Value = serde_json::from_slice(&frame).map_err(|e| Error::Protocol(e.to_string()))?;
        match v.get("ids").and_then(Value::as_array) {
            Some(ids) if v.get("type").and_then(Value::as_str) == Some("models") => {
                Ok(ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
            }
            _ => Err(Error::Protocol(format!("list_models reply {v}"))),
        }
    }

    #[allow(clippy::too_many_arguments)] // Matches the public streaming API.
    pub fn chat(
        endpoint: &Path,
        app: &str,
        model: &str,
        messages: &[Message],
        params: &Params,
        timeouts: Timeouts,
        cancel: &AtomicBool,
        on_chunk: impl FnMut(&str),
    ) -> Result<Summary, Error> {
        let request = encode(&chat_request(model, messages, params)?, MAX_FRAME_V2)?;
        let mut reader = connect(endpoint, app, VERSION_INFERENCE)?;
        reader.inner.write_all(&request)?;
        reader.inner.set_read_timeout(Some(POLL))?;
        let mut writer = reader.inner.try_clone()?;
        drive(&mut reader, cancel, timeouts, |b| writer.write_all(b), on_chunk)
    }
}

/// Whether a daemon accepts connections on `endpoint` right now.
pub fn is_listening(endpoint: &std::path::Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::net::UnixStream::connect(endpoint).is_ok();
    #[cfg(not(unix))]
    {
        let _ = endpoint;
        false
    }
}

/// Start the daemon unless something already listens on `endpoint`; see [`find_daemon`].
/// `socket_env` is handed to it as `SHAREDAI_SOCKET` (its parent is created, mode 0700).
pub fn ensure_running(endpoint: &std::path::Path, socket_env: Option<&std::path::Path>) -> Result<(), Error> {
    #[cfg(unix)]
    return net::ensure_running(endpoint, socket_env);
    #[cfg(not(unix))]
    {
        let _ = (endpoint, socket_env);
        Err(Error::Unsupported)
    }
}

/// Register the model file at `path` (absolute) under `id`, with its size and SHA-256.
pub fn register_file(endpoint: &std::path::Path, app: &str, id: &str, path: &std::path::Path) -> Result<(), Error> {
    #[cfg(unix)]
    return net::register_file(endpoint, app, id, path);
    #[cfg(not(unix))]
    {
        let _ = (endpoint, app, id, path);
        Err(Error::Unsupported)
    }
}

/// The ids of the models registered with the daemon.
pub fn list_models(endpoint: &std::path::Path, app: &str) -> Result<Vec<String>, Error> {
    #[cfg(unix)]
    return net::list_models(endpoint, app);
    #[cfg(not(unix))]
    {
        let _ = (endpoint, app);
        Err(Error::Unsupported)
    }
}

/// Stream a chat reply from `model`, formatted with the model's own chat template. Blocks until
/// the reply ends; run it on a worker thread.
#[allow(clippy::too_many_arguments)]
pub fn chat(
    endpoint: &std::path::Path,
    app: &str,
    model: &str,
    messages: &[Message],
    params: &Params,
    timeouts: Timeouts,
    cancel: &AtomicBool,
    on_chunk: impl FnMut(&str),
) -> Result<Summary, Error> {
    #[cfg(unix)]
    return net::chat(endpoint, app, model, messages, params, timeouts, cancel, on_chunk);
    #[cfg(not(unix))]
    {
        let _ = (endpoint, app, model, messages, params, timeouts, cancel, on_chunk);
        Err(Error::Unsupported)
    }
}

#[cfg(test)]
mod tests;

/// Discover registered language models. No files are copied or models downloaded.
pub fn available_text_models() -> Result<Vec<String>, Error> {
    let endpoint = endpoint("")?;
    ensure_running(&endpoint, Some(&endpoint))?;
    let models = list_models(&endpoint, "nisaba")?;
    Ok(models.into_iter().filter(|id| select_text_model(std::slice::from_ref(id), "writing").is_some()).collect())
}
/// Route a writing or code request through Enki using the installed model appropriate to it.
pub fn chat_auto(task: &str, messages: &[Message], params: &Params, cancel: &AtomicBool, on_chunk: impl FnMut(&str)) -> Result<Summary, Error> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let endpoint = endpoint("")?;
    ensure_running(&endpoint, Some(&endpoint))?;
    let models = list_models(&endpoint, "nisaba")?;
    let model = select_text_model(&models, task)
        .ok_or_else(|| Error::Invalid("The local assistant is not ready. Configure text generation in Enki and retry.".into()))?;
    chat(&endpoint, "nisaba", &model, messages, params, Timeouts::default(), cancel, on_chunk)
}
