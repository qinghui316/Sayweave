# Validation storage

Storage maintenance owns build caches and explicitly registered, isolated validation environments. It does not own application data, credentials, installed components, Release assets, dependencies, toolchains or the shared Harness.

## Inspect and apply

`npm run storage:plan` is read-only for inspected files and writes a timestamped plan under `.local/storage/plans`. Inspect its deleted candidates, preserved entries and blocked reasons. `npm run storage:apply -- .local/storage/plans/<UUID>.json` explicitly applies that plan. Plans expire after 15 minutes. Each item is independently checked again; changed, linked, occupied or pinned items are skipped and recorded in `.local/storage/cleanup-<UUID>.json`.

Only direct Cargo incremental units and terminal, owned UUID validation environments are eligible. Native PowerShell verifies absolute containment, every ancestor and descendant for reparse points, active compilers/process references and exclusive file access. Legacy acceptance folders without an ownership record remain protected until their references and completion evidence are reconciled. Worktrees require the Harness detach workflow before removal.

`.local/storage/pins.json` is an array of `{ "relative": ".local/component-auto", "reason": "current candidate and unresolved diagnostics" }` records. Ancestors and descendants of a pin are protected. Valid reports, recovery anchors and unresolved cases stay pinned; exceeding a budget does not override a pin.

## Budgets and lifecycle

The combined incremental budget is 10 GiB. `scripts/storage-cargo.mjs` keeps incremental compilation enabled, reclaims idle old units before/after its Cargo child, and stores the latest 30 timing records. Direct Cargo invocations retain their standard behavior; use the wrapper or the storage plan after them. `npm run test:native` uses the wrapper.

Owned temporary environments have a 12 GiB combined working budget, installer retention a 3 GiB budget, compact reports a 2 GiB budget and each compact report a 64 MiB ceiling. At least 2 GiB of disk space is reserved. Active reservations count toward the working budget. Unsafe excess refuses another large allocation and reports the inspection command.

Model, speech and dictionary desktop entrypoints register their fresh UUID directories before allocation. On successful completion they first preserve the report and screenshots outside the environment, confirm process and credential settlement, then request native cleanup. Failed environments retain the newest three rounds for up to seven days; explicit pins and uncertain cleanup take precedence. A dead owner is marked interrupted on the next entry and kept pinned until native resources and credentials are reconciled. Existing profiles supplied to a live dashboard remain reference data and are not adopted for cleanup.

Only owned isolated directories are removable. A `--regular` runner's real application data and credentials remain outside those directories. Failed diagnostic installers and recovery records remain available until resolved.

## Artifacts and affected validation

Installer retention uses `.local/storage/installers/<SHA256>/installer.exe`, its original signature and a receipt listing product commits, purposes and original paths. An existing hash is checked and reused. Exporting a complete CI artifact is separate from local retention; historic referenced paths are preserved. Signature validation remains with the existing updater/release verifier.

Temporary CI diagnostics retain seven days, reusable tools/packages fourteen days, and compact receipts/evidence thirty days. Download unresolved failure artifacts before runner destruction or expiry. Formal Release assets keep their separate lifecycle.

Controller, documentation or storage changes do not invalidate a signed product merely because the commit changed. Compare affected product files/configuration, retain the original commit and hashes, and record which check actually changed. A native installer, component or release-profile change requires affected product proof. The present dev/test profile change leaves release settings unchanged.

Dev/test use `debug=1` and `incremental=true`; full local variable/type debugging is available with `CARGO_PROFILE_DEV_DEBUG=2` and `CARGO_PROFILE_TEST_DEBUG=2`. Cargo documents the debug levels and profile inheritance in the [official profile reference](https://doc.rust-lang.org/cargo/reference/profiles.html). Changing a development profile may cause a one-time cache rebuild; it does not rebuild an existing signed release package.

Space trends and warm compile timings are observed during normal subsequent development. Storage validation uses small isolated fixtures and source/configuration checks; it does not launch an additional large installation or update acceptance solely to measure growth.
