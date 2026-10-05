# Windows inline composition — Stage A

This is an **in-progress feasibility component**, not an enabled Sayweave feature.
The application still uses its existing final-delivery path. ASR streaming,
Session/Turn integration and automatic editing resume are gated on real Codex
and input-method coexistence evidence.

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

`test:tsf` owns a disposable RichEdit window, loads the actual COM DLL through
its factory export and activates it only in that process. It asserts actual text after cumulative updates, commit,
selection replacement and abort, plus expired-request refusal and handoff after
a property-only edit, multiline/emoji, and unloading with no live leases. It leaves the clipboard and existing input profiles
untouched. This **does not prove global registration, Codex, IME coexistence or
speech recognition integration**.

## Operator probe and installation scope

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

The initial build covers x64 only. x86 installation, upgrade/replacement, crash
recovery, product enablement and event-driven idle transport belong to later
stages after the feasibility gate. The prototype's 50ms apartment timer is not
an accepted production idle-resource result.
