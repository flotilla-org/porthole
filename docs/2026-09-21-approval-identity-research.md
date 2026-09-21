# Identity evidence for cross-host approval

Research date: 2026-09-21. Resolves the factual investigation in
[Establish trustworthy requester and operator identity for cross-host approval](https://github.com/flotilla-org/porthole/issues/164).
This is a bounded source review, not a choice of threat model or a new live
experiment. Porthole sources were inspected at `2e62cc1`; the
[existing experiment](../apps/macos/AuthorizationPrototype/NOTES.md) supplies the
only execution evidence. No services, credentials or permissions were changed.

## What the current paths identify

| Hop or claim | Evidence available now | Limit |
|---|---|---|
| Agent calling Porthole | **Verified token possession:** bearer secret matches a non-revoked stored token and identity. | Does not identify the executable, process occurrence, person or remote host presenting it. |
| Agent display name and metadata | **Caller-supplied labels**, stored at identity creation. | No verification of the named application or human. |
| Operator approving in the current TUI | **Local endpoint access**, not a separate operator credential. | An agent with that access can call the same management routes. |
| Mechanism calling the prototype broker | **Kernel-reported peer UID 0** via `getpeereid`. | Any qualifying root peer passes; neither the mechanism executable nor the original requester is authenticated. |
| Beneficiary of the custom OS right | The waiting Authorization Services invocation receives the result. | The broker has no verified beneficiary account/process or motivating operation in its request. |
| Kiwi signing a decision | **Signature verifies against the configured P-256 public key**; the experiment used a Secure Enclave key. | Public-key verification alone does not attest hardware protection, authentication method or human review. |
| Destination named “comte” | SSH transport was used; signed challenge contains a **hostname label**. | The protocol has no enrolled host/service key, and the signer does not authenticate the challenge's origin. |

These classifications follow the [agent guard](../crates/portholed/src/routes/agent_guard.rs),
[token store](../crates/portholed/src/agent_store.rs),
[management handlers](../crates/portholed/src/routes/agent_permissions.rs),
[router](../crates/portholed/src/server.rs), and
[prototype broker/signer](../apps/macos/AuthorizationPrototype/Probe.swift).
The Unix transport binds a socket and serves the router without attaching a
verified process identity to requests. Filesystem access and token policy are
separate checks. [Transport source](../crates/porthole-transport/src/lib.rs).

The current self-grant path is explicitly acknowledged by
[the deferred-authority ADR](adr/0006-agent-permission-authority-deferred.md).
Consequently, improving request presentation or adding biometrics to one client
cannot establish operator separation while alternate management calls remain
available under the same authority.

## Direct XPC gives evidence about the direct sender

Apple's public `xpc_connection_set_peer_code_signing_requirement`, available
since macOS 12, checks messages against a code-signing requirement. The installed
SDK's `usr/include/xpc/connection.h` states that rejected listener messages are
dropped and failed reply checks report
`XPC_ERROR_PEER_CODE_SIGNING_REQUIREMENT`. It can validate an allowed helper or
client without converting a caller-supplied PID into authority.
[API](https://developer.apple.com/documentation/xpc/xpc_connection_set_peer_code_signing_requirement(_:_:)).

For a received XPC dictionary, `SecCodeCreateWithXPCMessage` obtains a dynamic
code object using the associated audit token; `SecCodeCheckValidity` can then
check an explicit requirement. This is documented in the installed public
`Security.framework/Headers/SecCode.h`, and both APIs are public.
[Message identity](https://developer.apple.com/documentation/security/seccodecreatewithxpcmessage(_:_:_:)),
[validity check](https://developer.apple.com/documentation/security/seccodecheckvalidity(_:_:_:)).

These identify different things:

- **Account identity:** XPC's EUID/EGID describe the peer's effective account.
  An audit-session identifier describes its session, not a person who clicked.
- **Code identity:** a requirement can constrain identifier and signing chain.
  An asserted bundle ID, filesystem path or application title is not that check.
  A team-only requirement allows more code than a requirement naming one product.
- **Process occurrence:** repeated launches of the same signed executable have
  the same code identity. Message-associated evidence avoids a separate PID
  lookup, but does not by itself define a persistent launch identity for a
  grant covering future messages, reconnects or `exec`.

Apple documents the peer attributes and requirement syntax separately.
[XPC peer information](https://developer.apple.com/documentation/xpc/xpc-connections),
[code-signing requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).
The persistent process-occurrence claim needs a selected API contract and tests;
this review does not establish a portable audit-token equality protocol.

**Inference:** authenticating a signed launcher proves which launcher sent a
message. It does not prove which script, model session or upstream user caused
that launcher to act. Delegation through a helper must carry separately
validated initiator evidence, or attribute the request to the helper and mark
upstream attribution as asserted. Permitting a same-UID agent to launch the
approved CLI does not distinguish that agent from its operator.

## Authorization Services intermediaries lose the direct path

The mechanism's IPC peer is the authorization host. Authenticating that peer,
even by code signature, does not make its original client the broker's direct
peer. The prototype sends only `{"command":"request"}`.
[Mechanism source](../apps/macos/AuthorizationPrototype/Mechanism.c).

Apple's published authd source records separate client and authorization-creator
identities internally. However, `_set_process_hints` places client PID/UID in
ordinary hints, and `_set_auth_token_hints` places creator PID and audit token
there too. Later, `engine_authorize` copies the caller's environment into that
hint dictionary. `auth_items_copy` copies entries through `auth_items_add_item`;
these are not immutable, kernel-delivered peer credentials at the plug-in.
The separate immutable hints shown here contain Apple-signing booleans, not a
general original-client audit token.
[Hint construction](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/engine.m#L235-L267),
[environment copy](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/engine.m#L1618-L1620),
[copy implementation](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/authd/authitems.c#L615-L627).

**Conclusion from source, not an exploit test:** do not authorize a named original
app from those ordinary hints. The tags are also in Apple's private
[AuthorizationTagsPriv.h](https://github.com/apple-oss-distributions/Security/blob/db15acbe6a7f257a859ad9a3bb86097bfe0679d9/OSX/libsecurity_authorization/lib/AuthorizationTagsPriv.h).
No supported, trustworthy original-requester handoff to this prototype mechanism
was established. An isolated test could check actual hint substitution on a
particular OS; success would still need a supported-contract review before
relying on it. A cooperative executor with direct authenticated IPC is a
different integration path, not evidence that a transparent plug-in already
has that information.

## Remote hosts, operator keys and review

SSH authenticates its transport endpoints and remote login account according
to its configured authentication and host-key policy. Forwarding a Unix socket
creates a connection on the remote machine; the destination socket's local peer
credentials do not identify the originating agent. SSH login authority also
says nothing about permission to approve a particular Porthole scope.
[OpenSSH manual](https://man.openbsd.org/ssh.1).

The prototype verifies an enrolled public key, the exact pending payload,
decision, deadline and single-use state. It has one key for one dummy right,
not a delegation registry. The hostname is included in signed bytes, but is
accepted from the challenge: signing a string does not verify who supplied it.
The signer checks protocol/right/expiry and takes its decision from CLI arguments.
It has no authenticated request-fetch or independent review screen.
[Broker and signer source](../apps/macos/AuthorizationPrototype/Probe.swift).

The successful Touch ID experiment establishes that this key could sign and
complete the custom right. Its `.userPresence` access control permits biometry
or device authentication; it is not biometric-only. A signature does not encode
which method was used, and the broker accepts any matching P-256 public key
without attestation. The verifier therefore depends on trusted enrollment and
signer behavior for claims about fresh authentication and meaningful review.
[Evidence](../apps/macos/AuthorizationPrototype/NOTES.md),
[Apple access-control flags](https://developer.apple.com/documentation/security/secaccesscontrolcreateflags),
[Secure Enclave protection](https://developer.apple.com/documentation/security/protecting-keys-with-the-secure-enclave).

**Design implications, not settled policy:** enrollment must associate keys with
an operator or host/service and permitted scopes, including who can change that
association. Key possession, delegation and freshness are separate claims.
The review client must obtain authentic request contents and bind the decision
to the same revision it displays. Merely keeping a private key non-exportable
does not prevent an untrusted caller from asking the signer to sign misleading
contents. A terminal controlled by that caller cannot independently establish
what the operator saw. Whether a trusted native review screen is required
should follow the selected threat model.

## Substitution acceptance matrix

“Passes” below means the identified check accepts it, not that a live attack was
attempted or that every later operation succeeds.

| Substitution | Present check | Result / missing boundary |
|---|---|---|
| Another program presents a copied valid agent token | Token hash and revocation | Passes as that agent; no process binding. |
| Same-UID agent calls the approval management route | Local endpoint access | Can self-approve in current design; documented placeholder. |
| Another root process sends the prototype request | Peer UID equals zero | Passes; original app remains unknown. |
| Another binary merely claims the expected bundle ID | Hypothetical explicit XPC signing requirement | Claim alone fails; requires actual valid signing identity. |
| Another instance of the correctly signed CLI connects | Same hypothetical signing requirement | Passes code check; no unique workflow identity. |
| Caller substitutes ordinary client/creator hints | Published authd hint-copy path | Must treat as untrusted; actual OS test outstanding. |
| Response payload/decision changed, or response replayed | Prototype signature and pending-state checks | Rejected in recorded protocol tests; real remote replay also rejected. |
| Untrusted host supplies a fresh challenge labelled “comte” | Prototype signer checks syntax/right/expiry | Origin is unchecked; operator signature must not be interpreted as host authentication. |

The remaining conversational choice is which failures the first release must
resist: mistaken cooperative agents, hostile remote principals, hostile local
same-UID processes, or compromised execution hosts. The present launch and
management model separates agents from their operator only by convention.
Choosing stronger boundaries requires protecting the authority, signer and
executor from the actors they constrain; neither a renamed principal nor a
new approval UI supplies that protection.
