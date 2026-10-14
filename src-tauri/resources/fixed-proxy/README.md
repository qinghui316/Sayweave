# Fixed ordinary TSF proxy (ABI 1)

`abi1.dll` is the project's own proxy, extracted offline from the authenticated
v0.1.12 installer. `manifest.json` binds the complete product commit, installer,
both embedded EXEs, byte offsets, platform, ABI and the full ordinary proxy source
closure (including shared client/wire/settlement code). The baseline core metadata
is only for the isolated old-to-new core gate.

The normal desktop and release builders validate this resource before building
the core. A missing, altered or incompatible resource stops the build. A local
cache or supplied DLL cannot replace it. Private engine changes do not rebuild it.

An intentional proxy upgrade requires a reviewed new binary and provenance
manifest, refreshed source closure, ordinary-export/ABI validation and an explicit
host-restart migration decision. Do not regenerate the manifest to hide a source
change. Frozen public release assets are never rewritten.

The release gate downloads the original installer, verifies its full signature
with the retained native verifier, and extracts matching core/proxy bytes from
both EXEs. It uses only a fresh disposable GitHub-hosted Windows runner for
protected registration and actual same-process input; it refuses a local run or
an existing installation. This does not replace the owner's same-Codex upgrade
acceptance.

License: the repository's LICENSE and NOTICE apply; source is the recorded
`qinghui316/Sayweave` commit/tag, not an external binary dependency.
