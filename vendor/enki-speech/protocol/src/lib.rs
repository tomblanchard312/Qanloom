pub mod memory;
pub use memory::{Memory, MemoryTurn, VERSION_MEMORY};
use serde::{Deserialize, Serialize};
pub mod default_image;
pub use default_image::{
    CPU_IMAGE_MODEL_ID, GPU_IMAGE_MODEL_ID, IMAGE_IDLE_TIMEOUT, IMAGE_WALL_TIMEOUT, ImageCatalog, ImageFile, ImageProfile, image_catalog,
    image_files, image_profile,
};
pub use default_image::{
    DEFAULT_IMAGE_BUNDLE, DEFAULT_IMAGE_CFG, DEFAULT_IMAGE_FILES, DEFAULT_IMAGE_LICENSE, DEFAULT_IMAGE_MODEL_ID, DEFAULT_IMAGE_SIDE,
    DEFAULT_IMAGE_STEPS, DefaultImageFile, DefaultImageRole, user_models_dir,
};
/// The base protocol: health, lifecycle and the model registry.
pub const VERSION: u32 = 1;
/// Adds inference: generate, chat, embed and cancel (docs/DECISIONS.md D1).
pub const VERSION_INFERENCE: u32 = 2;
/// Adds image generation through stable-diffusion.cpp (docs/IMAGES.md).
pub const VERSION_IMAGE: u32 = 3;
/// Every version this build speaks, oldest first. Negotiation picks the newest in common.
pub const VERSION_SPEECH: u32 = 4;
pub const VERSION_CAPTIONS: u32 = 5;
pub const SUPPORTED_VERSIONS: [u32; 5] = [1, 2, 3, 4, 5];

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u32,
        app: String,
    },
    Negotiate {
        versions: Vec<u32>,
        app: String,
    },
    /// v3: shared preferences, conversation summary and completed turns.
    GetMemory,
    /// Compare-and-swap; stale revisions cannot overwrite another app's memory.
    UpdateMemory {
        memory: Memory,
    },
    /// Atomically append a completed exchange; never silently evicts history.
    AppendMemory {
        turn: MemoryTurn,
    },
    Shutdown,
    /// Release an idle worker and acknowledge only after its process exits.
    ReleaseModel {
        model: String,
    },
    Health,
    ListModels,
    /// Query the daemon recommendation without loading model weights.
    ImageDefaults {
        #[serde(default)]
        cpu: Option<bool>,
    },
    ImageCatalog {
        model: String,
    },
    RegisterModels {
        models: Vec<Model>,
    },
    GetModel {
        id: String,
    },
    RemoveModel {
        id: String,
    },
    /// v2: stream a completion of a raw prompt.
    Generate {
        model: String,
        prompt: String,
        #[serde(default)]
        params: GenerateParams,
    },
    /// v2: stream a reply to a conversation, formatted with the model's own chat template.
    Chat {
        model: String,
        messages: Vec<ChatMessage>,
        #[serde(default)]
        params: GenerateParams,
    },
    /// v2: one embedding vector per input.
    Embed {
        model: String,
        inputs: Vec<String>,
    },
    /// v2: stop the generation running on this connection.
    Cancel,
    /// v4: transcribe a user-selected audio file with a registered whisper.cpp model.
    TranscribeFile {
        model: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        /// Use whisper.cpp's speech-to-English translation mode.
        #[serde(default)]
        translate_to_english: bool,
    },
    /// v4: report local speech provider availability.
    SpeechCapabilities,
    /// v5: start an explicit, caller-consented live caption session.
    StartCaptionSession {
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        consent_granted: bool,
    },
    /// v5: submit bounded 16 kHz mono signed PCM. The caller owns microphone capture,
    /// device permissions, resampling and visible capture indication.
    CaptionAudioChunk {
        session_id: String,
        sequence: u64,
        start_ms: u64,
        samples: Vec<i16>,
    },
    /// v5: close a live caption session and discard its in-memory state.
    EndCaptionSession {
        session_id: String,
    },
    /// v5: report active caption sessions and the daemon's PCM input contract.
    CaptionStatus,
    /// v3: load a diffusion model and leave it resident. No picture is drawn.
    Prepare {
        model: String,
    },
    /// v3: text-to-image, a logo, or a generative expand. One image, then the connection closes.
    Imagine {
        model: String,
        task: ImageTask,
        prompt: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        negative: String,
        #[serde(default)]
        params: ImageParams,
        /// PNG, standard base64. Required for [`ImageTask::Expand`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_png: Option<String>,
    },
}

/// What an [`Request::Imagine`] request produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageTask {
    /// A picture from the prompt.
    Image,
    /// A picture from the prompt, with the logo prompt and negative prompt applied.
    Logo,
    /// Grow `source_png` by the expand margins and paint the new pixels.
    Expand,
}

/// Sampling controls for an image. Unset fields take the daemon's defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfg: Option<f32>,
    /// Negative means the daemon chooses one and reports it on the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    /// How far an expand may leave the original pixels. Ignored for a new image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expand_left: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expand_right: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expand_top: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expand_bottom: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A diffusion checkpoint: `.safetensors`, `.ckpt`, `.sft`, or any model file under a
/// `diffusion` directory (including a diffusion GGUF). Language models stay `.gguf` elsewhere.
pub fn is_diffusion_model(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    name.ends_with(".safetensors")
        || name.ends_with(".ckpt")
        || name.ends_with(".sft")
        || lower.contains("/diffusion/")
        || lower.contains("\\diffusion\\")
}

/// Sampling and length controls. Unset fields take the daemon's defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// 0 is greedy (deterministic). Default 0.8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    /// Generation ends before any of these strings would be emitted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// "system", "user" or "assistant".
    pub role: String,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model ended its turn.
    Stop,
    /// `max_tokens` or the context was reached.
    Length,
    /// A `stop` string was reached.
    StopSequence,
    Cancelled,
    /// The daemon's wall-time cap was reached.
    TimeLimit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceErrorKind {
    /// No registered model has this id.
    ModelNotFound,
    /// The model file failed verification (path, ownership, permissions, size or SHA-256).
    ModelRejected,
    /// The worker could not load the model.
    LoadFailed,
    /// The worker stopped unexpectedly; the next request starts a new one.
    WorkerCrashed,
    /// Parameters out of range, too many inputs, or a request this connection's version lacks.
    InvalidRequest,
    /// The prompt does not fit the context.
    ContextOverflow,
    /// No inference backend is installed.
    Unavailable,
    Internal,
}

/// v2 request limits, enforced by the daemon.
pub mod limits {
    pub const MAX_TOKENS: u32 = 4096;
    pub const MAX_STOP_SEQUENCES: usize = 8;
    pub const MAX_CHAT_MESSAGES: usize = 256;
    pub const MAX_EMBED_INPUTS: usize = 32;
    pub const MAX_EMBED_INPUT_CHARS: usize = 16384;
    pub const MIN_IMAGE_SIDE: u32 = 64;
    pub const MAX_IMAGE_SIDE: u32 = 1024;
    pub const MAX_IMAGE_STEPS: u32 = 50;
    pub const MAX_IMAGE_PROMPT_CHARS: usize = 2000;
    pub const MAX_SOURCE_PNG_BYTES: usize = 5 << 20;
    pub const MAX_EXPAND_PIXELS: u32 = 512;
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello {
        version: u32,
    },
    Negotiated {
        version: u32,
        process_id: u32,
    },
    Incompatible {
        supported: Vec<u32>,
    },
    Memory {
        memory: Box<Memory>,
    },
    MemoryError {
        kind: MemoryErrorKind,
    },
    Stopping,
    ModelReleased {
        model: String,
    },
    Health {
        status: String,
    },
    Models {
        ids: Vec<String>,
    },
    Model {
        model: Option<Model>,
    },
    Registered,
    Removed {
        existed: bool,
    },
    RegistryError {
        kind: RegistryErrorKind,
    },
    Error {
        message: String,
    },
    /// v2: part of a streamed generation. `seq` counts from 0 without gaps.
    Chunk {
        seq: u64,
        text: String,
    },
    /// v2: the terminal frame of a successful stream.
    Done {
        reason: StopReason,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    Embeddings {
        vectors: Vec<Vec<f32>>,
    },
    /// v2: the queue is full; nothing was started.
    Busy {
        retry_after_ms: u64,
    },
    InferenceError {
        kind: InferenceErrorKind,
        message: String,
    },
    /// v4 completed file transcription. Confidence is omitted because the CLI adapter does not
    /// expose a stable calibrated segment confidence value.
    Transcript {
        language: Option<String>,
        translated_to_english: bool,
        segments: Vec<SpeechSegment>,
    },
    SpeechCapabilities {
        available: bool,
        provider: String,
        reason: Option<String>,
    },
    CaptionSessionStarted {
        session_id: String,
    },
    /// A finalized result for one submitted chunk. Whisper.cpp's CLI does not expose
    /// calibrated confidence or stable partial hypotheses.
    Caption {
        session_id: String,
        sequence: u64,
        start_ms: u64,
        end_ms: u64,
        text: String,
        finalized: bool,
    },
    CaptionSessionEnded {
        session_id: String,
    },
    CaptionStatus {
        active_sessions: usize,
        max_sessions: usize,
        sample_rate_hz: u32,
        channels: u16,
        sample_format: String,
    },
    SpeechError {
        kind: SpeechErrorKind,
        message: String,
    },
    ImageDefaults {
        profile: ImageProfile,
    },
    ImageCatalog {
        catalog: ImageCatalog,
    },
    /// v3: one sampling step. `seq` counts from 0 without gaps.
    ImageStep {
        seq: u64,
        step: u32,
        total: u32,
    },
    /// v3: the model is loaded and waiting. `accelerators` is empty when it loaded on the CPU.
    Prepared {
        accelerators: Vec<String>,
    },
    /// v3: the terminal frame of a successful image request. `png` is standard base64.
    Image {
        png: String,
        width: u32,
        height: u32,
        seed: i64,
    },
}

/// Metadata only: registration never reads, downloads or executes the model file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub source: String,
    pub license: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryErrorKind {
    InvalidMetadata,
    Capacity,
    Busy,
    Storage,
}

/// Maximum JSON payload size, excluding the newline delimiter.
pub const MAX_FRAME_BYTES: usize = 65536;
/// Maximum payload once a connection has negotiated version 2 (prompts, embeddings).
pub const MAX_FRAME_BYTES_V2: usize = 4 << 20;

/// Maximum payload for version 3 images, including base64-encoded 1024px RGBA PNGs.
pub const MAX_FRAME_BYTES_V3: usize = 8 << 20;

/// The frame limit for a negotiated version.
pub fn frame_limit(version: u32) -> usize {
    if version == VERSION_IMAGE {
        MAX_FRAME_BYTES_V3
    } else if version >= VERSION_INFERENCE {
        MAX_FRAME_BYTES_V2
    } else {
        MAX_FRAME_BYTES
    }
}

/// Read one bounded newline-delimited frame, rejecting truncated frames.
pub async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    read_frame_limited(reader, MAX_FRAME_BYTES).await
}

/// [`read_frame`] with an explicit payload limit.
pub async fn read_frame_limited<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    use tokio::io::AsyncBufReadExt;
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if frame.is_empty() { Ok(None) } else { Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Truncated frame")) };
        }
        let end = available.iter().position(|byte| *byte == b'\n');
        let count = end.unwrap_or(available.len());
        if count > limit - frame.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Frame too large"));
        }
        // Reserve only after checking the limit, never grow beyond the payload cap.
        frame.reserve_exact(count);
        frame.extend_from_slice(&available[..count]);
        reader.consume(count + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(Some(frame));
        }
    }
}

/// Where the Unix daemon listens: `SHAREDAI_SOCKET` when set, else
/// `$XDG_RUNTIME_DIR/sharedai/ipc.sock`. The flag is true for the default path, whose
/// `sharedai` directory the daemon may create (mode 0700) inside the runtime directory.
#[cfg(unix)]
pub fn unix_endpoint() -> Option<(std::path::PathBuf, bool)> {
    unix_endpoint_from(std::env::var_os("SHAREDAI_SOCKET"), std::env::var_os("XDG_RUNTIME_DIR"))
}

/// [`unix_endpoint`] from explicit values. Empty and relative runtime directories are ignored,
/// as the XDG Base Directory specification requires.
pub fn unix_endpoint_from(socket: Option<std::ffi::OsString>, runtime_dir: Option<std::ffi::OsString>) -> Option<(std::path::PathBuf, bool)> {
    if let Some(socket) = socket.filter(|s| !s.is_empty()) {
        return Some((socket.into(), false));
    }
    let dir = std::path::PathBuf::from(runtime_dir.filter(|d| !d.is_empty())?);
    dir.is_absolute().then(|| (dir.join("sharedai").join("ipc.sock"), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_prefers_the_variable_then_the_runtime_directory() {
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        assert_eq!(unix_endpoint_from(os("/tmp/x/ipc.sock"), os("/run/user/1000")), Some(("/tmp/x/ipc.sock".into(), false)));
        assert_eq!(unix_endpoint_from(os(""), os("/run/user/1000")), Some(("/run/user/1000/sharedai/ipc.sock".into(), true)));
        assert_eq!(unix_endpoint_from(None, os("relative/dir")), None);
        assert_eq!(unix_endpoint_from(None, os("")), None);
        assert_eq!(unix_endpoint_from(None, None), None);
    }
    use tokio::io::BufReader;

    #[tokio::test]
    async fn bounded_frames() {
        for size in [0, MAX_FRAME_BYTES - 1, MAX_FRAME_BYTES] {
            let mut input = vec![b'x'; size];
            input.push(b'\n');
            assert_eq!(read_frame(&mut BufReader::new(input.as_slice())).await.unwrap().unwrap().len(), size);
        }
        for newline in [false, true] {
            let mut input = vec![b'x'; MAX_FRAME_BYTES + 1];
            if newline {
                input.push(b'\n');
            }
            assert_eq!(read_frame(&mut BufReader::new(input.as_slice())).await.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        }
        assert!(read_frame(&mut &b""[..]).await.unwrap().is_none());
        assert!(read_frame(&mut &b"partial"[..]).await.is_err());
        let mut reader = &b"one\ntwo\n"[..];
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), b"one");
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), b"two");
    }

    #[tokio::test]
    async fn image_frames_are_larger_but_bounded() {
        let limit = frame_limit(VERSION_IMAGE);
        assert_eq!(limit, 8 << 20);
        assert_eq!(frame_limit(VERSION_INFERENCE), 4 << 20);
        let mut frame = vec![b'x'; limit];
        frame.push(b'\n');
        assert!(read_frame_limited(&mut BufReader::new(frame.as_slice()), limit).await.is_ok());
        frame.insert(0, b'x');
        assert!(read_frame_limited(&mut BufReader::new(frame.as_slice()), limit).await.is_err());
    }

    #[tokio::test]
    async fn version_two_frames_are_larger_but_bounded() {
        let mut big = vec![b'x'; MAX_FRAME_BYTES + 1];
        big.push(b'\n');
        let limit = frame_limit(VERSION_INFERENCE);
        assert!(read_frame_limited(&mut BufReader::new(big.as_slice()), limit).await.is_ok());
        assert!(read_frame_limited(&mut BufReader::new(big.as_slice()), frame_limit(VERSION)).await.is_err());
        let mut huge = vec![b'x'; MAX_FRAME_BYTES_V2 + 1];
        huge.push(b'\n');
        assert!(read_frame_limited(&mut BufReader::new(huge.as_slice()), limit).await.is_err());
    }

    #[test]
    fn inference_frames_match_the_documented_contract() {
        let r: Request = serde_json::from_str(r#"{"type":"generate","model":"m","prompt":"Hi","params":{"max_tokens":256}}"#).unwrap();
        assert!(matches!(r, Request::Generate { ref params, .. } if params.max_tokens == Some(256)));
        let r: Request = serde_json::from_str(r#"{"type":"generate","model":"m","prompt":"Hi"}"#).unwrap();
        assert!(matches!(r, Request::Generate { ref params, .. } if *params == GenerateParams::default()));
        assert_eq!(serde_json::to_string(&Response::Chunk { seq: 0, text: "Hel".into() }).unwrap(), r#"{"type":"chunk","seq":0,"text":"Hel"}"#);
        assert_eq!(
            serde_json::to_string(&Response::Done { reason: StopReason::Stop, prompt_tokens: 9, completion_tokens: 2 }).unwrap(),
            r#"{"type":"done","reason":"stop","prompt_tokens":9,"completion_tokens":2}"#
        );
        assert!(matches!(serde_json::from_str(r#"{"type":"cancel"}"#).unwrap(), Request::Cancel));
    }

    #[test]
    fn image_frames_and_diffusion_paths() {
        let r: Request = serde_json::from_str(r#"{"type":"imagine","model":"sd","task":"logo","prompt":"a mark"}"#).unwrap();
        assert!(matches!(r, Request::Imagine { task: ImageTask::Logo, .. }));
        let r: Request = serde_json::from_str(r#"{"type":"prepare","model":"flux-2-klein-4b-Q4_0"}"#).unwrap();
        assert!(matches!(r, Request::Prepare { .. }));
        assert_eq!(
            serde_json::to_string(&Response::Prepared { accelerators: vec!["Vulkan".into()] }).unwrap(),
            r#"{"type":"prepared","accelerators":["Vulkan"]}"#
        );
        assert!(is_diffusion_model("/models/v1-5.safetensors"));
        assert!(is_diffusion_model("/models/diffusion/flux.gguf"));
        assert!(!is_diffusion_model("/models/llama.gguf"));
    }
}

#[cfg(windows)]
pub mod windows;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeechErrorKind {
    InvalidRequest,
    ModelNotFound,
    ModelRejected,
    ProviderUnavailable,
    TranscriptionFailed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryErrorKind {
    Conflict,
    Capacity,
    Invalid,
    Storage,
}

#[cfg(test)]
mod speech_compat_tests {
    use super::*;
    #[test]
    fn speech_v4_is_additive_and_preserves_timestamp_fields() {
        assert_eq!(SUPPORTED_VERSIONS, [1, 2, 3, 4, 5]);
        assert_eq!(frame_limit(VERSION_SPEECH), MAX_FRAME_BYTES_V2);
        assert_eq!(frame_limit(VERSION_CAPTIONS), MAX_FRAME_BYTES_V2);
        let request = Request::TranscribeFile {
            model: "whisper-small".into(),
            path: "/tmp/selected.wav".into(),
            language: Some("en".into()),
            translate_to_english: false,
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"transcribe_file","model":"whisper-small","path":"/tmp/selected.wav","language":"en","translate_to_english":false}"#
        );
        let response = Response::Transcript {
            language: Some("en".into()),
            translated_to_english: false,
            segments: vec![SpeechSegment { start_ms: 125, end_ms: 900, text: "Hello".into() }],
        };
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(encoded.contains(r#""start_ms":125"#));
        assert!(encoded.contains(r#""end_ms":900"#));
        assert!(serde_json::from_str::<Response>(&encoded).is_ok());
    }

    #[test]
    fn caption_v5_frames_are_additive_and_bounded_by_audio_duration() {
        let request = Request::CaptionAudioChunk { session_id: "session-1".into(), sequence: 2, start_ms: 1_250, samples: vec![0, -1, 1] };
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(encoded, r#"{"type":"caption_audio_chunk","session_id":"session-1","sequence":2,"start_ms":1250,"samples":[0,-1,1]}"#);
        let response =
            Response::Caption { session_id: "session-1".into(), sequence: 2, start_ms: 1_250, end_ms: 1_500, text: "hello".into(), finalized: true };
        assert!(serde_json::from_str::<Response>(&serde_json::to_string(&response).unwrap()).is_ok());
        let status =
            Response::CaptionStatus { active_sessions: 1, max_sessions: 8, sample_rate_hz: 16_000, channels: 1, sample_format: "s16le".into() };
        assert!(serde_json::from_str::<Response>(&serde_json::to_string(&status).unwrap()).is_ok());
    }
}
