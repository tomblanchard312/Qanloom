//! Download a verified bundle from the installed Enki catalog into the user's models folder.
//!
//! The download runs only when an app asks for it. Bytes land in `*.part` and are renamed
//! after the SHA-256 matches, so a partial file is never registered. The VAE and the text
//! encoder sit in a subdirectory the scanner does not walk.

use sha2::{Digest, Sha256};
use sharedai_protocol::{DEFAULT_IMAGE_MODEL_ID, DefaultImageRole, ImageCatalog, ImageFile, Model, Request, Response, user_models_dir};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::{Cancellation, Client, Error, service_endpoint};

/// Result of a finished download. `registered` is false when the files are on disk and the
/// daemon is not running yet.
pub struct DownloadReport {
    pub model_id: String,
    pub registered: bool,
    pub message: String,
}

static DOWNLOAD: Mutex<()> = Mutex::new(());

/// True when every pinned file is present at its expected size and the bundle sidecar exists.
/// This does not rehash the files; the daemon checks the SHA-256 before it loads the model.
pub fn default_image_model_present() -> bool {
    image_model_present(DEFAULT_IMAGE_MODEL_ID)
}

pub fn image_model_present(model: &str) -> bool {
    let Some(files) = sharedai_protocol::image_files(model) else {
        return false;
    };
    let Some(root) = user_models_dir() else {
        return false;
    };
    let diffusion = root.join("diffusion");
    files.iter().all(|file| std::fs::metadata(diffusion.join(file.relative)).is_ok_and(|meta| meta.is_file() && meta.len() == file.size))
        && diffusion.join(format!("{model}.gguf.bundle.json")).is_file()
}

/// Fetch the pinned bundle. `progress` is called with bytes finished and the total.
pub fn download_default_image_model(progress: impl FnMut(u64, u64)) -> Result<DownloadReport, Error> {
    download_image_model(DEFAULT_IMAGE_MODEL_ID, progress)
}

pub fn download_image_model(model: &str, mut progress: impl FnMut(u64, u64)) -> Result<DownloadReport, Error> {
    let manifest = daemon_catalog(model)?;
    validate_catalog(model, &manifest)?;
    let files = &manifest.files;
    let model_id = model;
    let Ok(_guard) = DOWNLOAD.try_lock() else {
        return Err(Error::Remote("a model download is already running".into()));
    };
    let Some(root) = user_models_dir() else {
        return Err(Error::Remote("Set HOME or XDG_DATA_HOME so SharedAI knows where to store models.".into()));
    };
    let diffusion = root.join("diffusion");
    ensure_private(&diffusion)?;
    let (diffusion_file, vae, llm) = catalog(files)?;
    let total = files.iter().map(|file| file.size).sum();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(120))
        .redirects(8)
        .user_agent("SharedAI/0.1")
        .build();
    let mut done_before = 0u64;
    for file in files {
        let dest = diffusion.join(&file.relative);
        fetch_verified(&agent, &file.url, &dest, &file.sha256, file.size, |got| {
            progress(done_before + got, total);
        })?;
        done_before += file.size;
    }
    let bundle = serde_json::json!({
        "vae": vae.relative,
        "llm": llm.relative,
    });
    let bytes = serde_json::to_vec_pretty(&bundle).map_err(|e| Error::Remote(format!("cannot write the model bundle: {e}")))?;
    let bundle_path = diffusion.join(format!("{model}.gguf.bundle.json"));
    let staged = part_path(&bundle_path);
    std::fs::write(&staged, bytes)?;
    std::fs::rename(staged, bundle_path)?;
    let path = diffusion.join(&diffusion_file.relative).canonicalize()?;
    let registered = match register_diffusion(model_id, &manifest.license, &path, &diffusion_file.sha256, diffusion_file.size) {
        Ok(()) => true,
        Err(error) if daemon_unreachable(&error) => false,
        Err(error) => return Err(error),
    };
    let message = if registered { format!("{model} is ready.") } else { format!("{model} is on disk. Start Enki, then Refresh.") };
    Ok(DownloadReport { model_id: model_id.into(), registered, message })
}

fn daemon_catalog(model: &str) -> Result<ImageCatalog, Error> {
    let endpoint = service_endpoint().ok_or_else(|| Error::Remote("Start Enki before downloading a model.".into()))?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        match Client::new(endpoint, "enki-models").request(Request::ImageCatalog { model: model.into() }, &Cancellation::new()).await? {
            Response::ImageCatalog { catalog } => Ok(catalog),
            _ => Err(Error::UnexpectedResponse),
        }
    })
}

fn validate_catalog(model: &str, manifest: &ImageCatalog) -> Result<(), Error> {
    use std::path::Component;
    let files = &manifest.files;
    let safe = manifest.profile.model == model
        && !model.is_empty()
        && model.len() <= 128
        && model.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && manifest.license.len() <= 256
        && !manifest.license.is_empty()
        && files.len() == 3
        && files.iter().all(|f| {
            !f.relative.is_empty()
                && f.relative.len() < 512
                && !f.relative.contains('\\')
                && Path::new(&f.relative).components().all(|c| matches!(c, Component::Normal(_)))
                && f.url.starts_with("https://huggingface.co/")
                && f.sha256.len() == 64
                && f.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                && f.size > 0
                && f.size <= 128 * 1024 * 1024 * 1024
        })
        && [DefaultImageRole::Diffusion, DefaultImageRole::Vae, DefaultImageRole::TextEncoder]
            .iter()
            .all(|role| files.iter().filter(|f| f.role == *role).count() == 1)
        && files.iter().filter(|f| f.role == DefaultImageRole::Diffusion).all(|f| f.relative == format!("{model}.gguf"));
    if !safe {
        return Err(Error::Remote("Enki supplied an invalid or unpinned image catalog.".into()));
    }
    Ok(())
}

fn catalog(files: &[ImageFile]) -> Result<(&ImageFile, &ImageFile, &ImageFile), Error> {
    let mut diffusion = None;
    let mut vae = None;
    let mut llm = None;
    for file in files {
        match file.role {
            DefaultImageRole::Diffusion => diffusion = Some(file),
            DefaultImageRole::Vae => vae = Some(file),
            DefaultImageRole::TextEncoder => llm = Some(file),
        }
    }
    match (diffusion, vae, llm) {
        (Some(diffusion), Some(vae), Some(llm)) => Ok((diffusion, vae, llm)),
        _ => Err(Error::Remote("the default image model catalog is incomplete".into())),
    }
}

fn register_diffusion(id: &str, license: &str, path: &Path, sha256: &str, size: u64) -> Result<(), Error> {
    let endpoint = service_endpoint()
        .ok_or_else(|| Error::Remote("SharedAI has no endpoint. Set XDG_RUNTIME_DIR or SHAREDAI_SOCKET, and start sharedai-daemon.".into()))?;
    let size_bytes = i64::try_from(size).map_err(|_| Error::Remote("the model file is larger than the registry can store".into()))?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let client = Client::new(endpoint, "sharedai");
        let model = Model {
            id: id.into(),
            path: path.to_string_lossy().into_owned(),
            sha256: sha256.into(),
            size_bytes,
            source: "models-folder".into(),
            license: license.into(),
        };
        match client.request(Request::RegisterModels { models: vec![model] }, &Cancellation::new()).await? {
            Response::Registered => Ok(()),
            _ => Err(Error::UnexpectedResponse),
        }
    })
}

fn daemon_unreachable(error: &Error) -> bool {
    match error {
        Error::Io(_) | Error::Timeout => true,
        Error::Remote(message) => message.contains("no endpoint"),
        _ => false,
    }
}

pub(crate) fn fetch_verified(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    sha256: &str,
    size: u64,
    mut progress: impl FnMut(u64),
) -> Result<(), Error> {
    if !url_allowed(url) {
        return Err(Error::Remote("the model URL is not in the pinned catalog".into()));
    }
    if let Some(parent) = dest.parent() {
        ensure_private(parent)?;
    }
    if dest.is_file() && file_matches(dest, sha256, size)? {
        progress(size);
        return Ok(());
    }
    // Keep the previous file until the replacement is verified.

    let part = part_path(dest);
    let mut have = std::fs::metadata(&part).ok().filter(|meta| meta.is_file() && meta.len() <= size).map(|meta| meta.len()).unwrap_or(0);
    if std::fs::metadata(&part).is_ok_and(|meta| meta.len() > size) {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let response = {
        let mut request = agent.get(url);
        if have > 0 {
            request = request.set("Range", &format!("bytes={have}-"));
        }
        request.call().map_err(http_error)?
    };
    let status = response.status();
    let (mut file, mut hasher, mut got) = if status == 206 && have > 0 {
        let hasher = hash_prefix(&part, have)?;
        let file = std::fs::OpenOptions::new().append(true).open(&part)?;
        (file, hasher, have)
    } else if status == 200 {
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&part)?;
        (file, Sha256::new(), 0)
    } else {
        return Err(Error::Remote(format!("the model host returned HTTP {status}")));
    };
    let mut reader = response.into_reader();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if got.saturating_add(n as u64) > size {
            let _ = std::fs::remove_file(&part);
            return Err(Error::Remote("the download is larger than the pinned file".into()));
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        got += n as u64;
        progress(got);
    }
    file.sync_all()?;
    drop(file);
    if got != size || hex(&hasher.finalize()) != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(Error::Remote("the download did not match the pinned SHA-256, so it was discarded".into()));
    }
    std::fs::rename(&part, dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o644));
    }
    Ok(())
}

fn file_matches(path: &Path, sha256: &str, size: u64) -> Result<bool, Error> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() != size {
        return Ok(false);
    }
    Ok(sha256_file(path)? == sha256)
}

fn hash_prefix(path: &Path, len: u64) -> Result<Sha256, Error> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut left = len;
    let mut buf = vec![0u8; 1 << 20];
    while left > 0 {
        let take = left.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..take])?;
        if n == 0 {
            return Err(Error::Remote("the partial download is shorter than expected".into()));
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hasher)
}

fn sha256_file(path: &Path) -> Result<String, Error> {
    let hasher = hash_prefix(path, std::fs::metadata(path)?.len())?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

fn url_allowed(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        return !rest.is_empty() && !rest.chars().any(|c| c.is_control() || c.is_whitespace());
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    rest.starts_with("127.0.0.1:") || rest.starts_with("[::1]:") || rest.starts_with("localhost:")
}

fn http_error(error: ureq::Error) -> Error {
    match error {
        ureq::Error::Status(code, _) => Error::Remote(format!("the model host returned HTTP {code}")),
        ureq::Error::Transport(transport) => Error::Remote(format!("the model download stopped: {transport}")),
    }
}

/// Create `path` and tighten it, and its parents down through the SharedAI models folder, to
/// mode 0700. Directories above that folder are left as they are.
fn ensure_private(path: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Some(models) = user_models_dir() else {
            return Ok(());
        };
        let shared = models.parent().map(Path::to_path_buf);
        let mut dir = path.to_path_buf();
        loop {
            let in_models = dir.starts_with(&models) || shared.as_ref().is_some_and(|s| dir == *s);
            if in_models {
                let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
            }
            if shared.as_ref().is_some_and(|s| dir == *s) || !dir.starts_with(&models) || !dir.pop() {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(2)).timeout_read(Duration::from_secs(2)).build()
    }

    fn serve(body: Vec<u8>) -> String {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
            let mut buf = vec![0u8; 4096];
            let mut n = 0usize;
            while n < buf.len() {
                match sock.read(&mut buf[n..]) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        n += read;
                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let req = String::from_utf8_lossy(&buf[..n]);
            let (status, start, slice): (u16, usize, &[u8]) = if let Some(rest) = req.split("Range: bytes=").nth(1) {
                let start = rest.split(['-', '\r']).next().unwrap_or("0").parse().unwrap_or(0);
                let start = start.min(body.len());
                (206, start, &body[start..])
            } else {
                (200, 0, body.as_slice())
            };
            let header = if status == 206 {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                    slice.len(),
                    start,
                    body.len().saturating_sub(1),
                    body.len()
                )
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
            };
            let _ = sock.write_all(header.as_bytes());
            let _ = sock.write_all(slice);
        });
        format!("http://127.0.0.1:{port}/model")
    }

    fn digest(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn the_default_image_catalog_is_https_and_pinned() {
        assert_eq!(DEFAULT_IMAGE_MODEL_ID, "flux-2-klein-4b-Q4_0");
        assert_eq!(sharedai_protocol::DEFAULT_IMAGE_LICENSE, "Apache-2.0");
        let mut roles = Vec::new();
        let mut total = 0u64;
        for file in sharedai_protocol::DEFAULT_IMAGE_FILES {
            assert!(file.url.starts_with("https://huggingface.co/"));
            assert!(url_allowed(file.url));
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(file.size > 0);
            roles.push(file.role);
            total += file.size;
        }
        assert!(roles.contains(&DefaultImageRole::Diffusion));
        assert!(roles.contains(&DefaultImageRole::Vae));
        assert!(roles.contains(&DefaultImageRole::TextEncoder));
        assert!(total < 6_000_000_000);
        assert!(!url_allowed("file:///tmp/x"));
        assert!(!url_allowed("http://example.com/x"));
    }

    #[test]
    fn a_verified_download_keeps_matching_bytes_and_discards_the_rest() {
        let body = b"flux-bytes".to_vec();
        let hash = digest(&body);
        let url = serve(body.clone());
        let dir = std::env::temp_dir().join(format!("sai-dl-{}", std::process::id()));
        let dest = dir.join("nested").join("model.bin");
        let _ = std::fs::remove_dir_all(&dir);
        fetch_verified(&agent(), &url, &dest, &hash, body.len() as u64, |_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!part_path(&dest).exists());

        // A file that already matches is kept, so this call must not open a connection.
        fetch_verified(&agent(), "http://127.0.0.1:1/nope", &dest, &hash, body.len() as u64, |_| {}).unwrap();

        let bad = serve(b"not-the-file".to_vec());
        let wrong = dir.join("wrong.bin");
        let err = fetch_verified(&agent(), &bad, &wrong, &hash, 12, |_| {}).unwrap_err();
        assert!(err.to_string().contains("SHA-256"));
        assert!(!wrong.exists());
        assert!(!part_path(&wrong).exists());
        // A rejected replacement cannot erase a previously working bundle member.
        let replacement = serve(b"bad-new-model".to_vec());
        assert!(fetch_verified(&agent(), &replacement, &dest, &digest(b"new-model"), 13, |_| {}).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn daemon_catalog_rejects_paths_roles_and_unverified_files() {
        let id = sharedai_protocol::GPU_IMAGE_MODEL_ID;
        let mut manifest = sharedai_protocol::image_catalog(id).unwrap();
        assert!(validate_catalog(id, &manifest).is_ok());
        // A future daemon can add a version without updating this SDK catalog.
        let mut future = manifest.clone();
        future.profile.model = "next-image-v2".into();
        future.files[0].relative = "next-image-v2.gguf".into();
        assert!(validate_catalog("next-image-v2", &future).is_ok());
        manifest.files[1].relative = "../outside.gguf".into();
        assert!(validate_catalog(id, &manifest).is_err());
        manifest = sharedai_protocol::image_catalog(id).unwrap();
        manifest.files[1].role = DefaultImageRole::Vae;
        assert!(validate_catalog(id, &manifest).is_err());
        manifest = sharedai_protocol::image_catalog(id).unwrap();
        manifest.files[0].sha256.clear();
        assert!(validate_catalog(id, &manifest).is_err());
    }

    #[test]
    fn a_partial_file_resumes() {
        let body = b"0123456789abcdef".to_vec();
        let hash = digest(&body);
        let url = serve(body.clone());
        let dir = std::env::temp_dir().join(format!("sai-resume-{}", std::process::id()));
        let dest = dir.join("model.bin");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(part_path(&dest), &body[..6]).unwrap();
        fetch_verified(&agent(), &url, &dest, &hash, body.len() as u64, |_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
