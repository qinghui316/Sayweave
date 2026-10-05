# Windows inline composition

This branch connects native ASR updates to task-owned TSF drafts. The host
coordinates Session/Turn identities, editing handoff and quiet-period resume;
the component updates one composition range in the focused target. A real Codex
probe retained its final text. Full speech/shortcut and input-method coexistence
acceptance remains separate from that probe and from controlled text tests.

The COM DLL owns one focused TSF composition on the target apartment. The host
owns audio, recognition, credentials and history. The bounded pipe carries
task/turn/revision operations and acknowledgements; it never exposes an external
window or text-reading API to a renderer. A current-user host lease, client and
server PID/lifetime checks, request deadlines, connection generations and edit
ownership protect queued edits. Updates replace one owned range. Commit and
handoff do not append the final text again. Abort uses `ITfRangeBackup` only
while text, selection and property ownership remain valid.

## Checks

From the repository root, with the standard Windows toolchain available:

```powershell
npm run check:tsf
npm run test:tsf
```

`test:tsf` owns disposable RichEdit and legacy EDIT windows, loads the actual COM DLL through
its factory export and activates it only in that process. It asserts actual text after cumulative updates, commit,
selection replacement and abort, plus expired-request refusal and handoff after
a property-only edit, multiline/emoji, and unloading with no live leases. It leaves the clipboard and existing input profiles
untouched. Legacy EDIT asserts exact live composition-range text and, after commit,
the final control text; its committed WM_GETTEXT buffer can exclude active
composition. These observations are reported separately. This **does not prove global registration, Codex, IME coexistence or
speech recognition integration**.

## Product component lifecycle

The standard `desktop:dev` and `desktop:build` commands build and embed the DLL
in the application. Settings exposes fixed status, enable, disable, install and
remove operations. Enablement is per user; install/update/removal explicitly
request Windows administrator approval through the same EXE's fixed helper.
No renderer-supplied script, path or DLL is accepted and startup never elevates.

The machine module lives in a protected Program Files directory named by its
SHA256. Ownership, reparse points and ACLs are checked before mutation. Updates
retain old mapped modules and replace only an owned COM registration; failed
readback rolls that path back. A protected ledger permits exact cleanup after
applications release mapped versions. Missing owned files remain repairable;
access-denied and other I/O errors are not treated as missing. Removal unregisters
first and reports pending file cleanup for an explicit retry. Unknown files are
never recursively deleted. Maintenance holds the same admission guard as
dictation, including while an elevated helper is still running after a timeout.

## Historical Stage A operator probe

`tsf-probe register DLL` attempts current-user COM registration and the official
TSF profile registration. Existing registrations are refused. On the current
medium-integrity Windows environment, `Profiles.Register` returns `E_FAIL`;
the TSF machine registration key is not writable. `E_FAIL` alone does not name
the underlying cause. HKCU COM registration is not proof of a per-user TSF
installation.

The prepared `install-stage-a.ps1` is a **separate, explicit administrator
operation**. It requires a reviewed manifest with the initiating SID and exact
binary hashes, installs only below protected Program Files, registers the speech
category and provides exact-version removal. It is not executed automatically.
Machine installation requires approval beyond the original per-user scope.
Failed TSF cleanup retains the owned registration and binaries for retry. A
copy-stage failure removes only the files created by that invocation.

After installation, run `tsf-probe probe` as the normal user, focus an intended
blank input during its countdown, and inspect two cumulative updates before
commit. The probe compares the keyboard profile before and after speech
activation. Acknowledgements alone are not an actual-input acceptance result;
Codex must be observed. The probe deactivates its speech profile and releases its
host capability when it exits. Default keyboard selection is never changed.

`tsf-probe activation-check` enables and deactivates only the installed speech
profile, checks the keyboard identity before and after, and performs no text
input. The probe refuses to activate if another speech profile is already active;
enumeration failure also stops the probe. Cleanup is checked explicitly before
reporting success. `status` uses the per-profile enablement API; the profile
manager's `GetActiveProfile` API supports the keyboard category only.
An optional `probe <expected-pid> <focus-seconds>` countdown (5–30 seconds) is
operator preparation time, not product input latency.

The build covers x64 targets. x86 application compatibility remains unverified.
The component's 50ms apartment timer exists while its speech profile is active;
deactivation destroys its owned window and timer. Long-session and idle-resource
measurements, machine lifecycle runtime checks and original-environment IME
handoff still need their own evidence. Private-factory tests do not establish
those results.
