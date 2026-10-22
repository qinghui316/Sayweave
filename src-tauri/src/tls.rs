/// Select the existing provider before HTTP, WSS or updater initialization.
/// Updater's ring feature otherwise makes Rustls' implicit selection ambiguous.
pub(crate) fn initialize() {
    // An already installed process provider is retained on repeated initialization.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, time::Duration};
    use tokio::io::AsyncReadExt;

    #[test]
    fn cold_process_tls_without_prior_model_test_or_update_check() {
        const SCENARIO: &str = "SAYWEAVE_COLD_TLS_TEST";
        let Ok(scenario) = std::env::var(SCENARIO) else {
            for scenario in ["wss-first", "http-first"] {
                let output = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "tls::tests::cold_process_tls_without_prior_model_test_or_update_check",
                        "--nocapture",
                    ])
                    .env(SCENARIO, scenario)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{scenario}: stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        };
        // A shared test runner can hide this regression by installing a provider.
        assert!(rustls::crypto::CryptoProvider::get_default().is_none());
        initialize();
        let provider = rustls::crypto::CryptoProvider::get_default().unwrap();
        initialize();
        assert!(std::sync::Arc::ptr_eq(
            provider,
            rustls::crypto::CryptoProvider::get_default().unwrap()
        ));
        let http = || reqwest::Client::builder().build().unwrap();
        if scenario == "http-first" {
            drop(http());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut hello = [0u8; 1];
                tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut hello))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(hello[0], 0x16, "client must begin a TLS handshake");
                // Close without a certificate: production TLS must return an error.
            });
            let url = format!("wss://{address}");
            // Use the production WSS library's default native-roots connector,
            // with neither fixture trust nor an explicit per-connection provider.
            let error = tokio_tungstenite::connect_async(url.as_str())
                .await
                .expect_err("an unauthenticated TLS peer must not succeed");
            assert!(matches!(
                error,
                tokio_tungstenite::tungstenite::Error::Io(_)
                    | tokio_tungstenite::tungstenite::Error::Tls(_)
            ));
            server.await.unwrap();
        });
        drop(http());
    }
}
