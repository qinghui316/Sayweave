use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};
struct Reply {
    status: u16,
    body: String,
    headers: String,
    delay: Duration,
    chunked: bool,
    split_bytes: usize,
}
impl Reply {
    fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.into(),
            headers: String::new(),
            delay: Duration::ZERO,
            chunked: false,
            split_bytes: 0,
        }
    }
}
struct Fixture {
    url: String,
    provider: Arc<Provider>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    seen: Arc<Mutex<Vec<String>>>,
}
impl Fixture {
    fn start(replies: Vec<Reply>) -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into()),
            )
            .unwrap();
        let client = Provider::builder()
            .no_proxy()
            .tls_certs_only([reqwest::Certificate::from_der(cert.cert.der()).unwrap()])
            .build()
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let seen = Arc::new(Mutex::new(vec![]));
        let thread_seen = seen.clone();
        let handle = thread::spawn(move || {
            let mut replies = replies.into_iter();
            let mut reply = replies.next();
            while !thread_stop.load(Ordering::Acquire) && reply.is_some() {
                let stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let session = rustls::ServerConnection::new(Arc::new(config.clone())).unwrap();
                let mut stream = rustls::StreamOwned::new(session, stream);
                let Ok(request) = read_request(&mut stream) else {
                    continue;
                };
                thread_seen.lock().unwrap().push(request);
                let item = reply.take().unwrap();
                thread::sleep(item.delay);
                let header = if item.chunked {
                    format!(
                        "HTTP/1.1 {} OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n{}\r\n",
                        item.status, item.headers
                    )
                } else {
                    format!(
                        "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n{}\r\n",
                        item.status,
                        item.body.len(),
                        item.headers
                    )
                };
                let _ = stream.write_all(header.as_bytes());
                if item.chunked {
                    for chunk in item.body.as_bytes().chunks(if item.split_bytes == 0 {
                        item.body.len().max(1)
                    } else {
                        item.split_bytes
                    }) {
                        let _ = write!(stream, "{:x}\r\n", chunk.len());
                        let _ = stream.write_all(chunk);
                        let _ = stream.write_all(b"\r\n");
                        let _ = stream.flush();
                    }
                    let _ = stream.write_all(b"0\r\n\r\n");
                } else {
                    let _ = stream.write_all(item.body.as_bytes());
                }
                let _ = stream.flush();
                reply = replies.next();
            }
        });
        Self {
            url: format!("https://localhost:{port}/v1"),
            provider: Arc::new(Provider::with_client(client).unwrap()),
            stop,
            thread: Some(handle),
            seen,
        }
    }
    fn input(&self) -> ConfigInput {
        ConfigInput {
            name: String::new(),
            protocol: "openai-chat".into(),
            base_url: self.url.clone(),
            model_id: "test-model".into(),
            usages: vec![Usage::Text],
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
fn read_request(
    stream: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>,
) -> Result<String, ()> {
    let mut data = vec![];
    let end = loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(|_| ())?;
        data.push(byte[0]);
        if data.len() > 8192 {
            return Err(());
        }
        if data.ends_with(b"\r\n\r\n") {
            break data.len();
        }
    };
    let header = String::from_utf8_lossy(&data);
    let len = header
        .lines()
        .find_map(|s| {
            s.to_lowercase()
                .strip_prefix("content-length:")
                .and_then(|v| v.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    if len > 16 * 1024 {
        return Err(());
    }
    data.resize(end + len, 0);
    stream.read_exact(&mut data[end..]).map_err(|_| ())?;
    String::from_utf8(data).map_err(|_| ())
}
#[tokio::test]
async fn https_list_and_text_parameter_fallback() {
    let fixture = Fixture::start(vec![
        Reply::json(
            200,
            r#"{"data":[{"id":"alpha"},{"id":"alpha"},{"id":"beta"}]}"#,
        ),
        Reply::json(
            400,
            r#"{"error":{"param":"max_tokens","code":"unsupported_parameter","message":"unsupported"}}"#,
        ),
        Reply::json(200, r#"{"choices":[{"message":{"content":"OK"}}]}"#),
    ]);
    let input = fixture.input();
    assert_eq!(
        fixture
            .provider
            .models(&input, "dummy-fixture-key")
            .await
            .unwrap(),
        vec!["alpha", "beta"]
    );
    let result = fixture
        .provider
        .text(&input, "dummy-fixture-key")
        .await
        .unwrap();
    assert!(result.tested_at > 0);
    let seen = fixture.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen[0].starts_with("GET /v1/models "));
    let first: Value = serde_json::from_str(seen[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let second: Value = serde_json::from_str(seen[2].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(first["max_tokens"], 128);
    assert!(first.get("max_completion_tokens").is_none());
    assert_eq!(second["max_completion_tokens"], 128);
    assert!(second.get("max_tokens").is_none());
    assert_eq!(first["messages"][0]["content"], "Reply with OK.");
    assert_eq!(first.as_object().unwrap().len(), 4);
}
#[tokio::test]
async fn status_json_size_empty_response_and_redirect() {
    for (reply, code) in [
        (Reply::json(401, "raw secret provider response"), "auth"),
        (Reply::json(429, "{}"), "rateLimit"),
        (Reply::json(503, "{}"), "service"),
        (Reply::json(200, "not-json"), "protocol"),
        (
            Reply::json(200, r#"{"choices":[{"message":{"content":null}}]}"#),
            "emptyText",
        ),
        (
            Reply::json(200, r#"{"choices":[{"message":{"content":" "}}]}"#),
            "emptyText",
        ),
        (
            Reply::json(400, r#"{"error":{"message":"max_tokens unsupported"}}"#),
            "protocol",
        ),
    ] {
        let fixture = Fixture::start(vec![reply]);
        let error = fixture
            .provider
            .text(&fixture.input(), "dummy")
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, code);
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("raw secret")
        );
        assert_eq!(fixture.seen.lock().unwrap().len(), 1);
    }
    for chunked in [false, true] {
        let mut reply = Reply::json(200, &"x".repeat(LIMIT + 1));
        reply.chunked = chunked;
        let fixture = Fixture::start(vec![reply]);
        assert_eq!(
            fixture
                .provider
                .models(&fixture.input(), "dummy")
                .await
                .err()
                .unwrap()
                .code,
            "responseTooLarge"
        );
    }
    let mut reply = Reply::json(302, "");
    reply.headers = "Location: https://example.invalid/should-not-follow\r\n".into();
    let fixture = Fixture::start(vec![reply]);
    assert_eq!(
        fixture
            .provider
            .models(&fixture.input(), "dummy")
            .await
            .err()
            .unwrap()
            .code,
        "protocol"
    );
    assert_eq!(fixture.seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn request_capacity_cancel_before_start_and_deadline() {
    let provider =
        Arc::new(Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap());
    let ids: Vec<_> = (0..5).map(|_| uuid::Uuid::new_v4().to_string()).collect();
    let mut guards: Vec<_> = ids[..4]
        .iter()
        .map(|id| provider.requests.register(id).unwrap())
        .collect();
    assert_eq!(
        provider.requests.register(&ids[4]).err().unwrap().code,
        "busy"
    );
    provider.requests.cancel(&ids[0]).unwrap();
    assert_eq!(
        guards[0]
            .run(1, async { Ok::<_, ModelError>(()) })
            .await
            .err()
            .unwrap()
            .code,
        "cancelled"
    );
    let first = guards.remove(0);
    drop(first);
    let id = uuid::Uuid::new_v4().to_string();
    provider.requests.cancel(&id).unwrap();
    assert_eq!(
        provider.requests.register(&id).err().unwrap().code,
        "cancelled"
    );
    let mut guard = provider.requests.register(&ids[4]).unwrap();
    assert_eq!(
        guard
            .run(0, async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                Ok::<_, ModelError>(())
            })
            .await
            .err()
            .unwrap()
            .code,
        "timeout"
    );
    provider.requests.shutdown();
    assert_eq!(
        provider
            .requests
            .register(&uuid::Uuid::new_v4().to_string())
            .err()
            .unwrap()
            .code,
        "stopping"
    );
}
#[tokio::test]
async fn actual_cancel_stops_https_result_and_fallback_shares_deadline() {
    let mut first = Reply::json(
        400,
        r#"{"error":{"param":"max_tokens","code":"unsupported_parameter"}}"#,
    );
    first.delay = Duration::from_millis(650);
    let mut second = Reply::json(200, r#"{"choices":[{"message":{"content":"OK"}}]}"#);
    second.delay = Duration::from_millis(650);
    let fixture = Fixture::start(vec![first, second]);
    let mut guard = fixture
        .provider
        .requests
        .register(&uuid::Uuid::new_v4().to_string())
        .unwrap();
    let result = guard
        .run(1, fixture.provider.text(&fixture.input(), "dummy"))
        .await;
    assert_eq!(result.err().unwrap().code, "timeout");
    let mut reply = Reply::json(200, r#"{"data":[{"id":"alpha"}]}"#);
    reply.delay = Duration::from_millis(150);
    let fixture = Fixture::start(vec![reply]);
    let id = uuid::Uuid::new_v4().to_string();
    let mut guard = fixture.provider.requests.register(&id).unwrap();
    let provider = fixture.provider.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        provider.requests.cancel(&id).unwrap();
    });
    assert_eq!(
        guard
            .run(1, fixture.provider.models(&fixture.input(), "dummy"))
            .await
            .err()
            .unwrap()
            .code,
        "cancelled"
    );
}
#[tokio::test]
async fn normal_tls_verification_rejects_untrusted_fixture() {
    let fixture = Fixture::start(vec![Reply::json(200, r#"{"data":[]}"#)]);
    let provider = Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap();
    assert_eq!(
        provider
            .models(&fixture.input(), "dummy")
            .await
            .err()
            .unwrap()
            .code,
        "network"
    );
}

fn generated_sse(text: &str, finish: &str, done: bool) -> Reply {
    let mut body = format!(
        "data: {}\r\n\r\ndata: {}\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]})
    );
    if done {
        body.push_str("data: [DONE]\n\n");
    }
    let mut reply = Reply::json(200, &body);
    reply.headers = "Content-Type: text/event-stream\r\n".into();
    reply.chunked = true;
    reply.split_bytes = 1;
    reply
}
fn messages() -> Vec<Message> {
    vec![
        Message {
            role: "system",
            content: "Return only cleaned dictated text.".into(),
        },
        Message {
            role: "user",
            content: r#"{"input_text":"呃，中文🙂。"}"#.into(),
        },
    ]
}
#[tokio::test]
async fn generated_sse_and_json_complete_actual_tls_without_publishing_partial() {
    let fixture = Fixture::start(vec![
        generated_sse("中文🙂。", "stop", true),
        Reply::json(
            200,
            r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"中文🙂。"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        ),
    ]);
    let output = fixture
        .provider
        .generate(&fixture.input(), "fixture-key", &messages())
        .await
        .unwrap();
    assert_eq!(output.text, "中文🙂。");
    assert!(output.first_output_ms.is_some());
    let output = fixture
        .provider
        .generate(&fixture.input(), "fixture-key", &messages())
        .await
        .unwrap();
    assert_eq!(output.text, "中文🙂。");
    assert_eq!(output.usage.total, Some(15));
    let requests = fixture.seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let request: Value =
        serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(request["stream"], true);
    assert_eq!(request["model"], "test-model");
    assert_eq!(request["messages"][1]["content"], messages()[1].content);
}
#[tokio::test]
async fn generation_fallback_only_explicit_unsupported_parameter_and_incomplete_never_retries() {
    let fixture = Fixture::start(vec![
        Reply::json(
            400,
            r#"{"error":{"param":"max_tokens","code":"unsupported_parameter"}}"#,
        ),
        generated_sse("结果。", "stop", true),
    ]);
    assert_eq!(
        fixture
            .provider
            .generate(&fixture.input(), "fixture-key", &messages())
            .await
            .unwrap()
            .text,
        "结果。"
    );
    let requests = fixture.seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let retry: Value = serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(retry["max_completion_tokens"], 2048);
    drop(requests);
    for (reply, code) in [
        (generated_sse("partial", "stop", false), "incompleteText"),
        (generated_sse("partial", "length", true), "truncatedText"),
        (Reply::json(401, "secret provider text"), "auth"),
        (
            Reply::json(400, r#"{"error":{"message":"max_tokens unsupported"}}"#),
            "protocol",
        ),
    ] {
        let fixture = Fixture::start(vec![reply]);
        let error = fixture
            .provider
            .generate(&fixture.input(), "fixture-key", &messages())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, code);
        assert_eq!(fixture.seen.lock().unwrap().len(), 1);
        assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
    }
}
