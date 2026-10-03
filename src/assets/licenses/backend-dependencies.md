# Backend dependencies

The implementation is original Sayweave code. Reference projects are research sources, not vendored application code.

Direct dependencies added for model configuration are pinned in Cargo.toml / Cargo.lock:

| Crate | Version | License | Upstream |
|---|---|---|---|
| keyring | 3.6.3 | MIT OR Apache-2.0 | https://github.com/hwchen/keyring-rs |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | https://github.com/seanmonstar/reqwest |
| tauri-plugin-log | 2.8.0 | Apache-2.0 OR MIT | https://github.com/tauri-apps/plugins-workspace |

License identifiers and source URLs above were checked against the resolved crate manifests. Cargo.lock also records transitive dependencies; the table is not an exhaustive distribution notice. A distributable installer will require an audit of all resolved runtime dependencies and their complete notices. Sayweave's own license remains undecided as described in the repository LICENSE.

rcgen and rustls are test-only dependencies for controlled HTTPS fixtures, not an insecure production TLS implementation.
