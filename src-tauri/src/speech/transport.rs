use crate::history::types::Error;
use std::time::{Duration, Instant};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{http::Request, protocol::WebSocketConfig},
};
pub type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub async fn connect(base: &str, request: Request<()>) -> Result<Socket, Error> {
    let began = Instant::now();
    let connection_id = uuid::Uuid::new_v4();
    let mut phase = "dns";
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let connection = async {
        let host = request
            .uri()
            .host()
            .ok_or(Error::new("invalid"))?
            .trim_matches(['[', ']'])
            .to_owned();
        let port = request
            .uri()
            .port_u16()
            .unwrap_or(match request.uri().scheme_str() {
                Some("wss") => 443,
                Some("ws") => 80,
                _ => return Err(Error::new("invalid")),
            });
        let stage = Instant::now();
        let addresses = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| Error::new("network"))?
            .collect::<Vec<_>>();
        log::info!(target:"sayweave", "SAYWEAVE_SPEECH operation=transport_stage connection_id={} stage=dns elapsed_ms={} result=ok",connection_id,stage.elapsed().as_millis());
        phase = "tcp";
        let stage = Instant::now();
        let socket = tokio::net::TcpStream::connect(addresses.as_slice())
            .await
            .map_err(|_| Error::new("network"))?;
        // Small handshake/audio frames should leave immediately, without Nagle batching.
        socket
            .set_nodelay(true)
            .map_err(|_| Error::new("network"))?;
        log::info!(target:"sayweave", "SAYWEAVE_SPEECH operation=transport_stage connection_id={} stage=tcp elapsed_ms={} result=ok",connection_id,stage.elapsed().as_millis());
        phase = "tls_websocket";
        #[cfg(feature = "desktop-acceptance")]
        if let Some(connector) = acceptance_connector(base)? {
            return tokio_tungstenite::client_async_tls_with_config(
                request,
                socket,
                Some(config),
                Some(connector),
            )
            .await
            .map_err(connection_error);
        }
        let _ = base;
        tokio_tungstenite::client_async_tls_with_config(request, socket, Some(config), None)
            .await
            .map_err(connection_error)
    };
    let result = tokio::time::timeout(CONNECT_TIMEOUT, connection)
        .await
        .unwrap_or_else(|_| Err(Error::new("connectionTimeout")));
    log::info!(target:"sayweave", "SAYWEAVE_SPEECH operation=transport_connect connection_id={} stage={} elapsed_ms={} result={}",
        connection_id, phase, began.elapsed().as_millis(), result.as_ref().map(|_| "ok").unwrap_or_else(|e| e.code.as_str()));
    Ok(result?.0)
}

fn connection_error(e: tokio_tungstenite::tungstenite::Error) -> Error {
    match e {
        tokio_tungstenite::tungstenite::Error::Http(r) => {
            if r.headers()
                .get("X-Api-Status-Code")
                .is_some_and(|v| v == "45000030")
            {
                return Error::new("resourceUnavailable");
            }
            Error::new(match r.status().as_u16() {
                401 | 403 => "auth",
                429 => "rateLimit",
                _ => "protocol",
            })
        }
        _ => Error::new("network"),
    }
}
#[cfg(feature = "desktop-acceptance")]
fn acceptance_connector(base: &str) -> Result<Option<tokio_tungstenite::Connector>, Error> {
    let id = match std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID") {
        Ok(id) if uuid::Uuid::parse_str(&id).is_ok() => id,
        _ => return Ok(None),
    };
    if reqwest::Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .as_deref()
        != Some("localhost")
    {
        return Ok(None);
    }
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/data-acceptance")
        .join(id)
        .join("data/fixture-ca.der");
    let certificate = std::fs::read(file).map_err(|_| Error::new("protocol"))?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(certificate))
        .map_err(|_| Error::new("protocol"))?;
    Ok(Some(tokio_tungstenite::Connector::Rustls(
        std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    #[tokio::test]
    async fn handshake_can_complete_after_five_seconds_without_retry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_millis(5500)).await;
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let request = format!("ws://{address}").into_client_request().unwrap();
        let socket = connect("", request).await.unwrap();
        drop(socket);
        drop(server.await.unwrap());
    }

    #[tokio::test]
    async fn stalled_handshake_is_bounded_and_reports_connection_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let request = format!("ws://{address}").into_client_request().unwrap();
        let began = Instant::now();
        let error = connect("", request).await.err().unwrap();
        assert_eq!(error.code, "connectionTimeout");
        assert!(began.elapsed() >= CONNECT_TIMEOUT);
        assert!(began.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(5));
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn dropping_connect_future_does_not_wait_for_connection_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request = format!("ws://{address}").into_client_request().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), connect("", request))
                .await
                .is_err()
        );
    }
}
