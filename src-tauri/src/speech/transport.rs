use crate::history::types::Error;
use std::time::Duration;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{http::Request, protocol::WebSocketConfig},
};
pub type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
pub async fn connect(base: &str, request: Request<()>) -> Result<Socket, Error> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let connection = async {
        #[cfg(feature = "desktop-acceptance")]
        if let Some(connector) = acceptance_connector(base)? {
            return tokio_tungstenite::connect_async_tls_with_config(
                request,
                Some(config),
                false,
                Some(connector),
            )
            .await
            .map_err(connection_error);
        }
        let _ = base;
        tokio_tungstenite::connect_async_with_config(request, Some(config), false)
            .await
            .map_err(connection_error)
    };
    Ok(tokio::time::timeout(Duration::from_secs(5), connection)
        .await
        .map_err(|_| Error::new("timeout"))??
        .0)
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
