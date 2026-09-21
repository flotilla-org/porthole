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

Still unverified: loading the plug-in in authorizationhost, completing the custom
right through AuthorizationCopyRights, actual Touch ID signing, cross-host
transport, concurrent/deactivated engine callbacks and unload behavior. No system
right was registered, no plug-in installed, and comte was not contacted.

The demonstrated answer is limited: the challenge/signature and standing-policy
paths work as protocol mechanics. The main macOS integration question remains
for an explicit installation experiment. There is no authenticated original-app
identity, workload scoping, production resource policy or real privileged operation.
