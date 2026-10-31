# Fixed ordinary TSF proxy (ABI 1)

`abi1.dll` is the project's own reviewed ordinary proxy. Manifest version 2 binds
its exact bytes, full ordinary source closure (including shared client, wire and
settlement code), compiler output, locked build command and flags. The artifact
was produced from the recorded working-tree snapshot after `producerHead`; the
published source closure matches that snapshot. This is source-build provenance,
not a claim that an older signed installer contained the new proxy.

The independent `baseline` describes the authenticated v0.1.20 installer, both
extracted EXEs and the offsets of their old proxy and core. Version 1 manifests
remain supported for proxies originally extracted from a signed release.

The normal desktop and release builders validate this resource before building
the core. A missing, altered or incompatible resource stops the build. A local
cache or supplied DLL cannot replace it. Private engine changes do not rebuild it.

An intentional proxy upgrade requires a reviewed new binary and provenance
manifest, refreshed source closure, ordinary-export/ABI validation and an explicit
host-restart migration decision. Do not regenerate the manifest to hide a source
change. Frozen public release assets are never rewritten.

The release gate downloads the baseline installer, verifies its full signature
with the retained native verifier, and extracts matching core/proxy bytes from
both EXEs. It uses only a fresh disposable GitHub-hosted Windows runner for
protected registration, old-target refusal and reopened owned-target input. If
core bytes differ, it also verifies core switching within one new proxy instance.
It refuses a local run or an existing installation. This does not replace owner
installation and microphone acceptance. Users with an old proxy loaded normally
reopen that target application once; already matching proxy bytes do not require
another restart.

License: the repository's LICENSE and NOTICE apply; this is a Sayweave source
artifact, not an external binary dependency.
