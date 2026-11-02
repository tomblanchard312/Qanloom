#![cfg(unix)]
use sharedai_client::{Cancellation, Client, Error};
use sharedai_protocol::{Request, Response, read_frame};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::oneshot,
    time::{sleep, timeout},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Endpoint(PathBuf);
impl Endpoint {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let dir =
            std::env::temp_dir().canonicalize().unwrap().join(format!("sai-sdk-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        Self(dir.join("ipc.sock"))
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
async fn read(stream: &mut BufReader<UnixStream>) -> Request {
    serde_json::from_slice(&read_frame(stream).await.unwrap().unwrap()).unwrap()
}
async fn write(stream: &mut BufReader<UnixStream>, response: Response) {
    let mut data = serde_json::to_vec(&response).unwrap();
    data.push(b'\n');
    stream.write_all(&data).await.unwrap();
}
async fn negotiated(stream: &mut BufReader<UnixStream>) {
    assert!(matches!(read(stream).await, Request::Negotiate { versions, .. } if versions == vec![1]));
    write(stream, Response::Negotiated { version: 1, process_id: std::process::id() }).await;
}

#[tokio::test]
async fn legacy_hello_fallback_preserves_pr1_compatibility() {
    let endpoint = Endpoint::new();
    let listener = UnixListener::bind(&endpoint.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = BufReader::new(listener.accept().await.unwrap().0);
        assert!(matches!(read(&mut stream).await, Request::Negotiate { .. }));
        write(&mut stream, Response::Error { message: "Invalid request".into() }).await;
        assert!(matches!(read(&mut stream).await, Request::Hello { version: 1, .. }));
        write(&mut stream, Response::Hello { version: 1 }).await;
        assert!(matches!(read(&mut stream).await, Request::Health));
        write(&mut stream, Response::Health { status: "ok".into() }).await;
    });
    assert!(matches!(Client::new(&endpoint.0, "test").request(Request::Health, &Cancellation::new()).await.unwrap(), Response::Health { .. }));
    server.await.unwrap();
}

#[tokio::test]
async fn incompatible_and_unexpected_handshakes_send_no_operation() {
    for response in [
        Response::Incompatible { supported: vec![2] },
        Response::Negotiated { version: 999, process_id: 1 },
        Response::Health { status: "ok".into() },
    ] {
        let endpoint = Endpoint::new();
        let listener = UnixListener::bind(&endpoint.0).unwrap();
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(listener.accept().await.unwrap().0);
            let _ = read(&mut stream).await;
            write(&mut stream, response).await;
            assert!(read_frame(&mut stream).await.unwrap().is_none());
        });
        assert!(matches!(
            Client::new(&endpoint.0, "test").request(Request::Health, &Cancellation::new()).await,
            Err(Error::Incompatible { .. } | Error::UnexpectedResponse)
        ));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn retry_connects_to_a_delayed_server() {
    let endpoint = Endpoint::new();
    let path = endpoint.0.clone();
    let server = tokio::spawn(async move {
        sleep(Duration::from_millis(60)).await;
        let listener = UnixListener::bind(path).unwrap();
        let mut stream = BufReader::new(listener.accept().await.unwrap().0);
        negotiated(&mut stream).await;
        assert!(matches!(read(&mut stream).await, Request::Health));
        write(&mut stream, Response::Health { status: "ok".into() }).await;
    });
    assert!(matches!(Client::new(&endpoint.0, "test").request(Request::Health, &Cancellation::new()).await.unwrap(), Response::Health { .. }));
    server.await.unwrap();
}

#[tokio::test]
async fn operations_are_not_replayed_after_a_disconnect() {
    let endpoint = Endpoint::new();
    let listener = UnixListener::bind(&endpoint.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = BufReader::new(listener.accept().await.unwrap().0);
        negotiated(&mut stream).await;
        assert!(matches!(read(&mut stream).await, Request::Shutdown));
        drop(stream);
        assert!(timeout(Duration::from_millis(200), listener.accept()).await.is_err());
    });
    assert!(matches!(Client::new(&endpoint.0, "test").request(Request::Shutdown, &Cancellation::new()).await, Err(Error::Io(_))));
    server.await.unwrap();
}

#[tokio::test]
async fn cancellation_closes_handshake_and_response_connections() {
    for after_operation in [false, true] {
        let endpoint = Endpoint::new();
        let listener = UnixListener::bind(&endpoint.0).unwrap();
        let (ready, started) = oneshot::channel();
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(listener.accept().await.unwrap().0);
            if after_operation {
                negotiated(&mut stream).await;
                assert!(matches!(read(&mut stream).await, Request::Health));
            } else {
                let _ = read(&mut stream).await;
            }
            ready.send(()).unwrap();
            assert!(timeout(Duration::from_secs(1), read_frame(&mut stream)).await.unwrap().unwrap().is_none());
        });
        let cancellation = Cancellation::new();
        let clone = cancellation.clone();
        let client = Client::new(&endpoint.0, "test");
        let task = tokio::spawn(async move { client.request(Request::Health, &clone).await });
        started.await.unwrap();
        cancellation.cancel();
        assert!(matches!(task.await.unwrap(), Err(Error::Cancelled)));
        server.await.unwrap();
    }
    let endpoint = Endpoint::new();
    let cancellation = Cancellation::new();
    let clone = cancellation.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(10)).await;
        clone.cancel();
    });
    assert!(matches!(Client::new(&endpoint.0, "test").request(Request::Health, &cancellation).await, Err(Error::Cancelled)));
}

#[tokio::test]
async fn deadline_closes_stalled_handshake() {
    let endpoint = Endpoint::new();
    let listener = UnixListener::bind(&endpoint.0).unwrap();
    let server = tokio::spawn(async move {
        let mut stream = BufReader::new(listener.accept().await.unwrap().0);
        let _ = read(&mut stream).await;
        let mut byte = [0];
        assert_eq!(timeout(Duration::from_secs(6), stream.read(&mut byte)).await.unwrap().unwrap(), 0);
    });
    assert!(matches!(Client::new(&endpoint.0, "test").request(Request::Health, &Cancellation::new()).await, Err(Error::Timeout)));
    server.await.unwrap();
}

#[tokio::test]
async fn public_hello_and_negotiation_validate_the_operation_version() {
    for (request, response, accepted) in [
        (Request::Hello { version: 999, app: "test".into() }, Response::Hello { version: 1 }, false),
        (Request::Hello { version: 999, app: "test".into() }, Response::Hello { version: 999 }, true),
        (Request::Negotiate { versions: vec![999], app: "test".into() }, Response::Negotiated { version: 1, process_id: 1 }, false),
        (Request::Negotiate { versions: vec![999, 1], app: "test".into() }, Response::Negotiated { version: 1, process_id: 1 }, true),
    ] {
        let endpoint = Endpoint::new();
        let listener = UnixListener::bind(&endpoint.0).unwrap();
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(listener.accept().await.unwrap().0);
            negotiated(&mut stream).await;
            assert!(matches!(read(&mut stream).await, Request::Hello { .. } | Request::Negotiate { .. }));
            write(&mut stream, response).await;
        });
        let result = Client::new(&endpoint.0, "test").request(request, &Cancellation::new()).await;
        if accepted {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(Error::Incompatible { supported }) if supported == vec![1]));
        }
        server.await.unwrap();
    }
}
