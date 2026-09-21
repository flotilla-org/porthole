# Prototype evidence

2026-09-21, kiwi, macOS 26.6. Branch `prototype/remote-authorization` in
`/Users/robert/dev/porthole-authorization-prototype`, based on `8baef6a`.

The C mechanism and Swift probe compile with warnings treated as errors. Both
plists validate and the bundle exports AuthorizationPluginCreate. Staged binaries
were signed with Apple Development: Robert Wittams (DYYMCPD885), and codesign
verification passed. Rebuilding replaces those signed artifacts; sign again before
any system installation.

A disposable software-key protocol run verified signed allow and deny, rejection
of a modified decision, one-use/replay rejection, retirement on requester
disconnect, denial after the 60-second deadline, and the automatic policy path.
Processes, scratch sockets and demo keys were removed afterward.

Secure Enclave enrollment succeeded on kiwi with user-presence access control.
Its wrapped key and public key were removed afterward. No signing operation with
that key was attempted; this is **not** a successful Touch ID approval test.

The unchanged Rust workspace passed the required locked build, tests, all-target
Clippy with warnings denied, and pinned-nightly formatting checks. Logs are in
`/tmp/porthole-auth-prototype-{build,test,clippy,fmt}.log`.

At this initial stage, still unverified: loading the plug-in in authorizationhost, completing the custom
right through AuthorizationCopyRights, actual Touch ID signing, cross-host
transport, concurrent/deactivated engine callbacks and unload behavior. No system
right was registered, no plug-in installed, and comte was not contacted.

The demonstrated answer is limited: the challenge/signature and standing-policy
paths work as protocol mechanics. The main macOS integration question remains
for an explicit installation experiment. There is no authenticated original-app
identity, workload scoping, production resource policy or real privileged operation.

## Comte staging

Comte is reachable over SSH, Apple silicon, macOS 26.5.1 (25F80), user robert
UID 501. Noninteractive sudo requires a password. No pre-existing prototype
right, bundle or service files were found. Signed artifacts and the installer
are staged at `/tmp/porthole-auth-prototype.zbK6cy`; signatures verify remotely.
The CLI loads on comte and reports the expected missing-socket error.

An actual Secure Enclave operator key was enrolled on kiwi; its wrapped private
key stays on kiwi and only its public key was staged on comte. Scratch locations
are recorded in the ignored `.build/comte-state.json`. Installation is waiting
for administrator authentication in the user's own terminal at this stage. No
system changes had been made on comte yet.

The first installer attempt stopped at codesign: the inline `-R` requirements
were missing their leading `=`, so codesign interpreted them as filenames.
The exact verification command reproduced the failure locally. Both arguments
now have the prefix, and the installer's signature and right-plist checks pass
on comte. The corrected installer is staged at the same path. A read-only check
confirmed no installed prototype artifacts or custom right after the failed
attempt. All four workspace gates passed again; logs are in
`/tmp/porthole-auth-installer-{build,test,clippy,fmt}.log`.

## Installed experiment on comte

The user ran the corrected installer successfully. The ordinary-user request
returned `AuthorizationCopyRights(work.flotilla.prototype.remote-approval) = 0`.
The broker logged a standing-policy allow from peer UID 0. This verifies actual
plug-in loading and successful completion of the custom right, not just the
socket protocol. The before/after taskport policy snapshots compare equal.
The launchd job is now in human mode, with no `--automatic` argument.

A second real request produced a challenge on comte, fetched over SSH for the
Secure Enclave signer on kiwi. Signing did not complete within the harness's
55-second timeout; the signer was terminated and no decision was submitted.
The user reported not seeing a local authentication prompt.
The broker logged expiry at its 60-second deadline, and its pending list is empty.
At that point hardware-backed signing and remote signed approval remained
unverified. The installed custom right still grants no real privileged operation.

## Signed remote approval and missing-prompt diagnosis

The first attempt's logs show the system authentication UI reporting `DidAppear`
and `GotFocus`, then losing focus before the harness terminated the signer.
This does not explain why the user did not see it, but provides no evidence of
an identity rejection or blocked UI setup. Logs are captured in
`/tmp/porthole-auth-matching.log` on kiwi.

A local-only diagnostic challenge (never submitted to a broker) exercised the
same unmodified signer. A stack sample showed it waiting in synchronous
LocalAuthentication XPC. During sampling, Touch ID matched and the signer
produced a signed deny response. The sample is in
`/tmp/porthole-auth-sign.sample`; no signer code changes were needed.

The next full remote attempt succeeded: comte generated a fresh challenge,
kiwi signed allow with the enrolled Secure Enclave key, and the JSON response
was sent over SSH. Comte returned `ACCEPTED`, logged `HUMAN allow`, and the
ordinary-user request returned
`AuthorizationCopyRights(work.flotilla.prototype.remote-approval) = 0` (exit 0).
Resubmitting the identical response was rejected, and the pending list was empty.

Both automatic policy and hardware-backed signed remote approval now complete
the real custom Authorization Services right. The original prompt visibility
is unexplained; concurrent/deactivated callback and unload behavior, and real
OS-call results for signed denial/cancellation/timeout, remain unverified.
The broker remains installed in human mode on comte, with no pending request.
