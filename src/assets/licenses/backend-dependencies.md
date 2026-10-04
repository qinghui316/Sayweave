# Backend dependencies

The implementation is original Sayweave code. Reference projects are research sources, not vendored application code.

Direct dependencies added for model configuration are pinned in Cargo.toml / Cargo.lock:

| Crate | Version | License | Upstream |
|---|---|---|---|
| keyring | 3.6.3 | MIT OR Apache-2.0 | https://github.com/hwchen/keyring-rs |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| tauri-plugin-log | 2.8.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-opener | 2.7.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| cpal | 0.18.2 | Apache-2.0 | https://github.com/RustAudio/cpal |
| rubato | 5.0.1 | MIT OR Apache-2.0 | https://github.com/HEnquist/rubato |
| rtrb | 0.4.0 | MIT OR Apache-2.0 | https://github.com/mgeier/rtrb |
| hound | 3.5.1 | Apache-2.0 | https://github.com/ruuda/hound |
| rodio | 0.22.2 | MIT OR Apache-2.0 | https://github.com/RustAudio/rodio |
| tokio-tungstenite | 0.30.0 | MIT | https://github.com/snapview/tokio-tungstenite |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| tauri-plugin-clipboard-manager | 2.4.1 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-global-shortcut | 2.4.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |
| global-hotkey | 0.8.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/global-hotkey |
| windows / windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |

License identifiers and source URLs above were checked against the resolved crate manifests. Cargo.lock also records transitive dependencies; the table is not an exhaustive distribution notice. A distributable installer will require an audit of all resolved runtime dependencies and their complete notices. Sayweave's own license remains undecided as described in the repository LICENSE.

rcgen and the direct rustls test-trust adapter are limited to controlled fixtures (`desktop-acceptance` for native test trust). Production WebSocket TLS also uses rustls through tokio-tungstenite with normal certificate validation. futures-util 0.3 provides the stream/sink interfaces (MIT OR Apache-2.0); its resolved version is in Cargo.lock.
