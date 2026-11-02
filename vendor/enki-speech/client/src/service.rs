use super::*;
use tokio::process::{Child, Command};

/// An explicitly launched ordinary-user daemon. Drop kills only this owned child.
/// This does not install an OS service, modify startup settings, or attach to another daemon.
pub struct Service {
    child: Child,
    client: Client,
}
impl Service {
    pub async fn start(executable: &Path, endpoint: &Path, cancellation: &Cancellation) -> Result<Self, Error> {
        Self::launch(executable, endpoint, None, cancellation).await
    }

    /// Use an explicitly selected database in an existing private data directory.
    pub async fn start_with_registry(executable: &Path, endpoint: &Path, registry: &Path, cancellation: &Cancellation) -> Result<Self, Error> {
        Self::launch(executable, endpoint, Some(registry), cancellation).await
    }

    async fn launch(executable: &Path, endpoint: &Path, registry: Option<&Path>, cancellation: &Cancellation) -> Result<Self, Error> {
        if *cancellation.0.borrow() {
            return Err(Error::Cancelled);
        }
        let mut command = Command::new(executable);
        command.kill_on_drop(true).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(unix)]
        command.env("SHAREDAI_SOCKET", endpoint);
        #[cfg(windows)]
        {
            command.env("SHAREDAI_PIPE", endpoint);
            // CREATE_NO_WINDOW: a library caller does not need a console or elevation prompt.
            command.creation_flags(0x08000000);
        }
        if let Some(registry) = registry {
            command.env("SHAREDAI_REGISTRY", registry);
        }
        let mut child = command.spawn()?;
        let pid = child.id().ok_or(Error::ServiceExited)?;
        let client = Client::new(endpoint, "sharedai-lifecycle");
        let ready = bounded(cancellation, async {
            tokio::select! {
                biased;
                status = child.wait() => { status?; Err(Error::ServiceExited) },
                connected = async {
                    loop {
                        match client.connect().await {
                            Err(Error::Io(err)) if retryable(&err) => sleep(Duration::from_millis(25)).await,
                            other => break other,
                        }
                    }
                } => {
                    let (mut stream, owner) = connected?;
                    if owner != Some(pid) { return Err(Error::WrongInstance); }
                    stream.write_all(&encode(&Request::Health)?).await?;
                    match receive(&mut stream).await? {
                        Response::Health { status } if status == "ok" => Ok(()),
                        _ => Err(Error::UnexpectedResponse),
                    }
                }
            }
        })
        .await;
        if let Err(error) = ready {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        Ok(Self { child, client })
    }
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn stop(mut self, cancellation: &Cancellation) -> Result<(), Error> {
        // Verify ownership on the same connection used for shutdown: a replacement daemon is never stopped.
        let result = bounded(cancellation, async {
            let (mut stream, owner) = self.client.connect().await?;
            if owner != self.child.id() || owner.is_none() {
                return Err(Error::WrongInstance);
            }
            stream.write_all(&encode(&Request::Shutdown)?).await?;
            if !matches!(receive(&mut stream).await?, Response::Stopping) {
                return Err(Error::UnexpectedResponse);
            }
            if !self.child.wait().await?.success() {
                return Err(Error::ServiceExited);
            }
            Ok(())
        })
        .await;
        if result.is_err() {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        }
        result
    }
}
