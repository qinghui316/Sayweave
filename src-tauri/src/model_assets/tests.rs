use super::*;
use catalog::Descriptor;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{Mutex, Notify},
};
const DATA: &[u8] = b"fixture";
fn descriptor(id: &'static str, file: &'static str, url: String) -> Descriptor {
    Descriptor {
        id,
        file,
        bytes: DATA.len() as u64,
        sha256: Box::leak(format!("{:x}", Sha256::digest(DATA)).into_boxed_str()),
        sources: Box::leak(vec![&*Box::leak(url.into_boxed_str())].into_boxed_slice()),
    }
}
async fn server(
    body: &'static [u8],
    range: bool,
    hold: bool,
) -> (
    String,
    Arc<Mutex<String>>,
    Arc<Notify>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let request = Arc::new(Mutex::new(String::new()));
    let captured = request.clone();
    let sent = Arc::new(Notify::new());
    let signal = sent.clone();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = vec![];
        while !data.ends_with(b"\r\n\r\n") {
            let byte = socket.read_u8().await.unwrap();
            data.push(byte);
            assert!(data.len() < 4096);
        }
        *captured.lock().await = String::from_utf8(data).unwrap();
        let status = if range {
            "206 Partial Content"
        } else {
            "200 OK"
        };
        let content_range = if range {
            "Content-Range: bytes 3-6/7\r\n"
        } else {
            ""
        };
        let length = if hold { 7 } else { body.len() };
        socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nETag: \"v1\"\r\n{content_range}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
        socket.write_all(body).await.unwrap();
        signal.notify_one();
        if hold {
            std::future::pending::<()>().await;
        }
    });
    (url, request, sent, task)
}
async fn settled(assets: &Assets, id: &str) -> AssetStatus {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = assets.status(id).await.unwrap();
            if !matches!(state.state.as_str(), "downloading" | "verifying") {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registry_handles_two_assets_with_one_download_and_lease_contract() {
    let root = tempfile::tempdir().unwrap();
    let (url, _, _, peer) = server(DATA, false, false).await;
    let catalog = Box::leak(
        vec![
            descriptor("first", "first.gguf", url),
            descriptor("second", "second.gguf", "http://127.0.0.1:1/unused".into()),
        ]
        .into_boxed_slice(),
    );
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assets.download("first").await.unwrap();
    assert_eq!(settled(&assets, "first").await.state, "ready");
    peer.await.unwrap();
    std::fs::write(root.path().join("second.gguf"), DATA).unwrap();
    assert_eq!(assets.status("second").await.unwrap().state, "available");
    let lease = assets.lease("second").await.unwrap();
    assert!(lease.path.ends_with("second.gguf"));
    assert!(assets.delete("second").await.is_err());
    drop(lease);
    assets.delete("second").await.unwrap();
    assert_eq!(std::fs::read(root.path().join("first.gguf")).unwrap(), DATA);
    assets.shutdown().await;
    assert!(assets.lease("first").await.is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_only_a_pinned_range_and_digest_then_atomically_publishes() {
    let root = tempfile::tempdir().unwrap();
    let (url, request, _, peer) = server(b"ture", true, false).await;
    let catalog =
        Box::leak(vec![descriptor("first", "first.gguf", url.clone())].into_boxed_slice());
    std::fs::write(root.path().join("first.gguf.part"), b"fix").unwrap();
    std::fs::write(
        root.path().join("first.gguf.json"),
        serde_json::to_vec(&serde_json::json!({"source":url,"etag":"\"v1\""})).unwrap(),
    )
    .unwrap();
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assets.download("first").await.unwrap();
    assert_eq!(settled(&assets, "first").await.state, "ready");
    peer.await.unwrap();
    let request = request.lock().await.to_lowercase();
    assert!(request.contains("range: bytes=3-"));
    assert!(request.contains("if-range: \"v1\""));
    assert_eq!(std::fs::read(root.path().join("first.gguf")).unwrap(), DATA);
    assert!(!root.path().join("first.gguf.part").exists());
    assets.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_entity_discards_unsafe_resume_instead_of_looping_on_the_same_range() {
    let root = tempfile::tempdir().unwrap();
    let (url, _, _, peer) = server(b"ture", true, false).await;
    let catalog =
        Box::leak(vec![descriptor("first", "first.gguf", url.clone())].into_boxed_slice());
    std::fs::write(root.path().join("first.gguf.part"), b"fix").unwrap();
    let metadata = root.path().join("first.gguf.json");
    std::fs::write(
        &metadata,
        serde_json::to_vec(&serde_json::json!({"source":url,"etag":"\"v0\""})).unwrap(),
    )
    .unwrap();
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assets.download("first").await.unwrap();
    let result = settled(&assets, "first").await;
    assert_eq!(result.error.as_deref(), Some("localDownloadRange"));
    assert!(!metadata.exists());
    assert!(!root.path().join("first.gguf").exists());
    assert_eq!(
        std::fs::read(root.path().join("first.gguf.part")).unwrap(),
        b"fix"
    );
    peer.await.unwrap();
    assets.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn weak_entity_tag_restarts_download_without_appending_partial_bytes() {
    let root = tempfile::tempdir().unwrap();
    let (url, request, _, peer) = server(DATA, false, false).await;
    let catalog =
        Box::leak(vec![descriptor("first", "first.gguf", url.clone())].into_boxed_slice());
    std::fs::write(root.path().join("first.gguf.part"), b"old").unwrap();
    std::fs::write(
        root.path().join("first.gguf.json"),
        serde_json::to_vec(&serde_json::json!({"source":url,"etag":"W/\"v1\""})).unwrap(),
    )
    .unwrap();
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assets.download("first").await.unwrap();
    assert_eq!(settled(&assets, "first").await.state, "ready");
    assert!(!request.lock().await.to_lowercase().contains("range:"));
    assert_eq!(std::fs::read(root.path().join("first.gguf")).unwrap(), DATA);
    peer.await.unwrap();
    assets.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_digest_never_becomes_an_available_model() {
    let root = tempfile::tempdir().unwrap();
    let (url, _, _, peer) = server(b"f!xture", false, false).await;
    let catalog = Box::leak(vec![descriptor("first", "first.gguf", url)].into_boxed_slice());
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assets.download("first").await.unwrap();
    let state = settled(&assets, "first").await;
    assert_eq!(state.state, "failed");
    assert_eq!(state.error.as_deref(), Some("localModelCorrupt"));
    assert!(!root.path().join("first.gguf").exists());
    peer.await.unwrap();
    assets.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn damaged_existing_file_reports_failure_after_explicit_use() {
    let root = tempfile::tempdir().unwrap();
    let catalog = Box::leak(
        vec![descriptor(
            "first",
            "first.gguf",
            "http://127.0.0.1:1/unused".into(),
        )]
        .into_boxed_slice(),
    );
    std::fs::write(root.path().join("first.gguf"), b"f!xture").unwrap();
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assert_eq!(assets.status("first").await.unwrap().state, "available");
    assert_eq!(assets.lease("first").await.err(), Some("localModelCorrupt"));
    let state = assets.status("first").await.unwrap();
    assert_eq!(state.state, "failed");
    assert_eq!(state.error.as_deref(), Some("localModelCorrupt"));
    assets.shutdown().await;
}
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirected_asset_directory_cannot_write_or_remove_external_bytes() {
    let owner = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let linked = owner.path().join("models");
    // A Windows junction needs no symlink privilege and models the redirected directory.
    let output = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&linked)
        .arg(external.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    std::fs::write(external.path().join("first.gguf"), DATA).unwrap();
    let catalog = Box::leak(
        vec![descriptor(
            "first",
            "first.gguf",
            "http://127.0.0.1:1/unused".into(),
        )]
        .into_boxed_slice(),
    );
    let assets = Assets::with_catalog(linked.clone(), catalog).unwrap();
    assert_eq!(assets.lease("first").await.err(), Some("localFileAccess"));
    assert_eq!(assets.delete("first").await.err(), Some("localFileAccess"));
    assets.download("first").await.unwrap();
    assets.shutdown().await;
    assert_eq!(
        std::fs::read(external.path().join("first.gguf")).unwrap(),
        DATA
    );
    // Detach only the verified junction, then ordinary TempDir cleanup owns its directories.
    use std::os::windows::fs::MetadataExt;
    assert_ne!(
        std::fs::symlink_metadata(&linked)
            .unwrap()
            .file_attributes()
            & 0x400,
        0
    );
    std::fs::remove_dir(linked).unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_and_shutdown_close_the_download_before_releasing_ownership() {
    let root = tempfile::tempdir().unwrap();
    let (url, _, sent, peer) = server(b"fi", false, true).await;
    let catalog = Box::leak(vec![descriptor("first", "first.gguf", url)].into_boxed_slice());
    let assets = Assets::with_catalog(root.path().to_owned(), catalog).unwrap();
    assets.download("first").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), sent.notified())
        .await
        .unwrap();
    assert_eq!(assets.delete("first").await.err(), Some("busy"));
    assets.cancel("first").unwrap();
    assert_eq!(settled(&assets, "first").await.state, "missing");
    assets.delete("first").await.unwrap();
    assets.shutdown().await;
    peer.abort();
    let _ = peer.await;
    assert!(!root.path().join("first.gguf").exists());
}
