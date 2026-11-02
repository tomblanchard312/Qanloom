//! Local inference through the daemon (protocol version 2, docs/DECISIONS.md D1).

use crate::{Cancellation, Client, Connection, Error, open, retryable};
use sharedai_protocol::{
    ChatMessage, GenerateParams, ImageParams, ImageTask, InferenceErrorKind, Request, Response, StopReason, VERSION_IMAGE, VERSION_INFERENCE,
    frame_limit, read_frame_limited,
};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    time::{sleep, timeout},
};

/// How long to wait for each event of an inference stream. The first one may include loading
/// the model; later ones only one token.
#[derive(Clone, Copy, Debug)]
pub struct InferenceTimeouts {
    pub first: Duration,
    pub idle: Duration,
}

impl Default for InferenceTimeouts {
    fn default() -> Self {
        Self { first: Duration::from_secs(360), idle: Duration::from_secs(60) }
    }
}

/// What to draw. Width and height of 0 use the daemon default (512, a multiple of 8).
#[derive(Clone, Debug)]
pub struct ImageRequest {
    pub model: String,
    pub task: ImageTask,
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    /// Negative lets the daemon choose the seed.
    pub seed: i64,
    pub strength: f32,
    pub expand_left: u32,
    pub expand_right: u32,
    pub expand_top: u32,
    pub expand_bottom: u32,
    pub source_png: Option<Vec<u8>>,
}

impl ImageRequest {
    pub fn image(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::new(model, ImageTask::Image, prompt)
    }

    pub fn logo(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::new(model, ImageTask::Logo, prompt)
    }

    pub fn expand(model: impl Into<String>, prompt: impl Into<String>, png: Vec<u8>) -> Self {
        let mut request = Self::new(model, ImageTask::Expand, prompt);
        request.expand_left = 128;
        request.expand_right = 128;
        request.expand_top = 128;
        request.expand_bottom = 128;
        request.source_png = Some(png);
        request
    }

    fn new(model: impl Into<String>, task: ImageTask, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            task,
            prompt: prompt.into(),
            negative: String::new(),
            width: 0,
            height: 0,
            steps: 0,
            cfg: 0.0,
            seed: -1,
            strength: 0.0,
            expand_left: 0,
            expand_right: 0,
            expand_top: 0,
            expand_bottom: 0,
            source_png: None,
        }
    }
}

/// A finished image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub seed: i64,
}

/// How a finished generation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub reason: StopReason,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// A running generation: text arrives through [`Generation::next`] until it returns `None`;
/// then [`Generation::summary`] tells how it ended. Dropping it closes the connection, which
/// cancels the generation.
pub struct Generation {
    stream: Connection,
    timeouts: InferenceTimeouts,
    cancellation: Cancellation,
    next_seq: u64,
    summary: Option<Summary>,
}

impl Generation {
    /// The next piece of text, or `None` once the generation has finished.
    pub async fn next(&mut self) -> Result<Option<String>, Error> {
        if self.summary.is_some() {
            return Ok(None);
        }
        let wait = if self.next_seq == 0 { self.timeouts.first } else { self.timeouts.idle };
        let response = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => {
                let _ = self.stream.write_all(b"{\"type\":\"cancel\"}\n").await;
                return Err(Error::Cancelled);
            }
            r = timeout(wait, receive_frame(&mut self.stream, VERSION_INFERENCE)) => r.map_err(|_| Error::Timeout)??,
        };
        match response {
            Response::Chunk { seq, text } if seq == self.next_seq => {
                self.next_seq += 1;
                Ok(Some(text))
            }
            Response::Chunk { .. } => Err(Error::UnexpectedResponse),
            Response::Done { reason, prompt_tokens, completion_tokens } => {
                self.summary = Some(Summary { reason, prompt_tokens, completion_tokens });
                Ok(None)
            }
            other => Err(terminal_error(other)),
        }
    }

    /// Asks the daemon to stop; keep calling [`Generation::next`] to receive what was already
    /// sent and the summary (reason `cancelled`, or `stop` if it finished first).
    pub async fn cancel(&mut self) -> Result<(), Error> {
        self.stream.write_all(b"{\"type\":\"cancel\"}\n").await?;
        Ok(())
    }

    pub fn summary(&self) -> Option<Summary> {
        self.summary
    }

    /// Collects the rest of the text and the summary.
    pub async fn collect(mut self) -> Result<(String, Summary), Error> {
        let mut text = String::new();
        while let Some(piece) = self.next().await? {
            text.push_str(&piece);
        }
        let summary = self.summary.ok_or(Error::UnexpectedResponse)?;
        Ok((text, summary))
    }
}

fn terminal_error(response: Response) -> Error {
    match response {
        Response::Busy { retry_after_ms } => Error::Busy { retry_after: Duration::from_millis(retry_after_ms) },
        Response::InferenceError { kind, message } => Error::Inference { kind, message },
        Response::Error { message } => Error::Remote(message),
        Response::Incompatible { supported } => Error::Incompatible { supported },
        _ => Error::UnexpectedResponse,
    }
}

async fn receive_frame(stream: &mut Connection, version: u32) -> Result<Response, Error> {
    let frame = read_frame_limited(stream, frame_limit(version))
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "SharedAI closed the stream"))?;
    Ok(serde_json::from_slice(&frame)?)
}

fn encode_v2(request: &Request) -> Result<Vec<u8>, Error> {
    encode_frame(request, VERSION_INFERENCE)
}

fn encode_frame(request: &Request, version: u32) -> Result<Vec<u8>, Error> {
    let mut data = serde_json::to_vec(request)?;
    if data.len() > frame_limit(version) {
        return Err(Error::TooLarge);
    }
    data.push(b'\n');
    Ok(data)
}

impl Client {
    /// Uses these timeouts for generate, chat and embed.
    pub fn with_inference_timeouts(mut self, timeouts: InferenceTimeouts) -> Self {
        self.inference_timeouts = timeouts;
        self
    }

    /// Streams a completion of `prompt` from a registered model.
    pub async fn generate(&self, model: &str, prompt: &str, params: GenerateParams, cancellation: &Cancellation) -> Result<Generation, Error> {
        self.start(Request::Generate { model: model.into(), prompt: prompt.into(), params }, cancellation).await
    }

    /// Streams a reply to `messages`, formatted with the model's own chat template.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        params: GenerateParams,
        cancellation: &Cancellation,
    ) -> Result<Generation, Error> {
        self.start(Request::Chat { model: model.into(), messages, params }, cancellation).await
    }

    /// One embedding vector per input.
    pub async fn embed(&self, model: &str, inputs: Vec<String>, cancellation: &Cancellation) -> Result<Vec<Vec<f32>>, Error> {
        let data = encode_v2(&Request::Embed { model: model.into(), inputs })?;
        let first = self.inference_timeouts.first;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(Error::Cancelled),
            r = timeout(first, async {
                let mut stream = self.connect_inference().await?;
                stream.write_all(&data).await?;
                match receive_frame(&mut stream, VERSION_INFERENCE).await? {
                    Response::Embeddings { vectors } => Ok(vectors),
                    other => Err(terminal_error(other)),
                }
            }) => r.map_err(|_| Error::Timeout)?,
        }
    }

    async fn start(&self, request: Request, cancellation: &Cancellation) -> Result<Generation, Error> {
        let data = encode_v2(&request)?;
        let stream = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(Error::Cancelled),
            r = timeout(Duration::from_secs(5), async {
                let mut stream = self.connect_inference().await?;
                stream.write_all(&data).await?;
                Ok::<_, Error>(stream)
            }) => r.map_err(|_| Error::Timeout)??,
        };
        Ok(Generation { stream, timeouts: self.inference_timeouts, cancellation: cancellation.clone(), next_seq: 0, summary: None })
    }

    /// Load `model` and leave the worker resident. A diffusion model only.
    pub async fn prepare(&self, model: &str) -> Result<(), Error> {
        let data = encode_v2(&Request::Prepare { model: model.to_string() })?;
        let mut stream = timeout(Duration::from_secs(5), async {
            let mut stream = self.connect_inference_version(VERSION_IMAGE).await?;
            stream.write_all(&data).await?;
            Ok::<_, Error>(stream)
        })
        .await
        .map_err(|_| Error::Timeout)??;
        let response =
            timeout(sharedai_protocol::IMAGE_IDLE_TIMEOUT, receive_frame(&mut stream, VERSION_INFERENCE)).await.map_err(|_| Error::Timeout)??;
        match response {
            Response::Prepared { .. } => Ok(()),
            other => Err(terminal_error(other)),
        }
    }

    /// Draws one image. Zero parameters use the daemon's model-specific defaults.
    pub async fn imagine(&self, request: ImageRequest, cancellation: &Cancellation) -> Result<Picture, Error> {
        self.drive_image(request, cancellation, &mut |_, _| {}).await
    }

    /// [`imagine`](Self::imagine), calling `on_step` with each sampling step the daemon reports.
    /// The first call can be a long time after the request: that wait is the model loading.
    pub async fn imagine_with<F: FnMut(u32, u32) + Send>(
        &self,
        request: ImageRequest,
        cancellation: &Cancellation,
        on_step: &mut F,
    ) -> Result<Picture, Error> {
        self.drive_image(request, cancellation, on_step).await
    }

    async fn drive_image<F: FnMut(u32, u32) + Send>(
        &self,
        request: ImageRequest,
        cancellation: &Cancellation,
        on_step: &mut F,
    ) -> Result<Picture, Error> {
        let source_png = request
            .source_png
            .as_ref()
            .map(|png| {
                use base64::Engine;
                if png.len() > sharedai_protocol::limits::MAX_SOURCE_PNG_BYTES {
                    return Err(Error::TooLarge);
                }
                Ok(base64::engine::general_purpose::STANDARD.encode(png))
            })
            .transpose()?;
        let data = encode_frame(
            &Request::Imagine {
                model: request.model,
                task: request.task,
                prompt: request.prompt,
                negative: request.negative,
                params: ImageParams {
                    width: (request.width > 0).then_some(request.width),
                    height: (request.height > 0).then_some(request.height),
                    steps: (request.steps > 0).then_some(request.steps),
                    cfg: (request.cfg > 0.0).then_some(request.cfg),
                    seed: (request.seed >= 0).then_some(request.seed),
                    strength: (request.strength > 0.0).then_some(request.strength),
                    expand_left: request.expand_left,
                    expand_right: request.expand_right,
                    expand_top: request.expand_top,
                    expand_bottom: request.expand_bottom,
                },
                source_png,
            },
            VERSION_IMAGE,
        )?;
        let mut stream = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(Error::Cancelled),
            r = timeout(Duration::from_secs(5), async {
                let mut stream = self.connect_inference_version(VERSION_IMAGE).await?;
                stream.write_all(&data).await?;
                Ok::<_, Error>(stream)
            }) => r.map_err(|_| Error::Timeout)??,
        };
        let mut seq = 0u64;
        let mut first = true;
        loop {
            let wait = if first { sharedai_protocol::IMAGE_WALL_TIMEOUT } else { sharedai_protocol::IMAGE_IDLE_TIMEOUT };
            first = false;
            let response = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    let _ = stream.write_all(b"{\"type\":\"cancel\"}\n").await;
                    return Err(Error::Cancelled);
                }
                r = timeout(wait, receive_frame(&mut stream, VERSION_IMAGE)) => r.map_err(|_| Error::Timeout)??,
            };
            match response {
                Response::ImageStep { seq: got, step, total, .. } if got == seq => {
                    seq += 1;
                    on_step(step, total);
                }
                Response::ImageStep { .. } => return Err(Error::UnexpectedResponse),
                Response::Image { png, width, height, seed } => {
                    let png = decode_result_png(&png)?;
                    return Ok(Picture { png, width, height, seed });
                }
                Response::Done { reason: StopReason::Cancelled, .. } => return Err(Error::Cancelled),
                other => return Err(terminal_error(other)),
            }
        }
    }

    /// A connection that negotiated version 2. Retries stop before any operation is written.
    async fn connect_inference(&self) -> Result<Connection, Error> {
        self.connect_inference_version(VERSION_INFERENCE).await
    }

    async fn connect_inference_version(&self, wanted: u32) -> Result<Connection, Error> {
        let negotiation = encode_v2(&Request::Negotiate { versions: vec![wanted], app: self.app.clone() })?;
        for attempt in 0..=4 {
            let result = async {
                let mut stream = BufReader::new(open(&self.endpoint).await?);
                stream.write_all(&negotiation).await?;
                match receive_frame(&mut stream, VERSION_INFERENCE).await? {
                    Response::Negotiated { version, .. } if version == wanted => Ok(stream),
                    Response::Negotiated { version, .. } => Err(Error::Incompatible { supported: vec![version] }),
                    Response::Incompatible { supported } => Err(Error::Incompatible { supported }),
                    // A first-release daemon (Hello only) has no inference.
                    Response::Error { message } if message == "Invalid request" => {
                        Err(Error::Incompatible { supported: vec![sharedai_protocol::VERSION] })
                    }
                    other => Err(terminal_error(other)),
                }
            }
            .await;
            match result {
                Err(Error::Io(ref err)) if attempt < 4 && retryable(err) => sleep(Duration::from_millis(25 << attempt)).await,
                other => return other,
            }
        }
        Err(Error::Timeout)
    }
}

fn decode_result_png(text: &str) -> Result<Vec<u8>, Error> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|e| Error::Remote(format!("image data is not base64: {e}")))?;
    if bytes.len() > sharedai_protocol::limits::MAX_SOURCE_PNG_BYTES {
        return Err(Error::TooLarge);
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::Remote("image data is not a PNG".into()));
    }
    Ok(bytes)
}

impl Error {
    /// The inference error kind, if this is one.
    pub fn inference_kind(&self) -> Option<InferenceErrorKind> {
        match self {
            Error::Inference { kind, .. } => Some(*kind),
            _ => None,
        }
    }
}

#[cfg(test)]
#[test]
fn image_results_reject_png_bytes_beyond_the_limit() {
    use base64::Engine;
    let png = vec![0; sharedai_protocol::limits::MAX_SOURCE_PNG_BYTES + 1];
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    assert!(matches!(decode_result_png(&encoded), Err(Error::TooLarge)));
}
