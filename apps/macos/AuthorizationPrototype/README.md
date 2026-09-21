# Throwaway remote authorization probe

Question: can one custom Authorization Services right be completed either by a
signed decision from another machine, or by a standing local policy?

This is an integration probe, not production authority. The only right is
`work.flotilla.prototype.remote-approval`. It grants no privileged operation.
It never modifies taskport, login, TCC, or any existing system right.

Build from the repository root:

```sh
apps/macos/AuthorizationPrototype/build.sh
```

The outputs are `.build/authorization-probe` and
`.build/PortholeAuthorizationPrototype.bundle` inside this directory. No package
manager, daemon installation or system changes occur during build.

## Protocol before installation

Run the interactive software-key walkthrough from the repository root:

```sh
python3 apps/macos/AuthorizationPrototype/demo.py
```

It builds the probe, creates disposable keys/sockets, offers signed allow/deny
and automatic approval, then cleans up its processes and scratch files.


`enroll-demo KEY` creates a disposable software P-256 key and `KEY.pub`.
`serve-demo SOCKET KEY.pub` starts the broker as the current user.
`request-demo SOCKET` waits for a decision. In a second terminal,
`pending SOCKET` lists the base64 challenge payloads. Use
`sign-demo KEY PAYLOAD allow` (or `deny`) to write a JSON response, then
`submit SOCKET RESPONSE.json` to send it. The server verifies the enrolled key,
the exact challenge bytes and decision, its 60-second deadline and one-use state.

`serve-demo SOCKET KEY.pub --automatic` instead allows the custom probe request
immediately. Output identifies a policy decision separately from human approval.
This flag grants the entire **custom test right**. It does not implement workload
or original-requester restrictions. Never use this policy for an existing right.

Each command prints its state/result. Demo commands do not use Authorization
Services or prove Touch ID, plugin loading, remote networking or taskport access.
They deliberately have separate names; the real signer never falls back to a
software key. Stop brokers with Ctrl-C and remove their scratch sockets/keys.
The broker refuses to replace an existing socket path.

## Human authentication on kiwi

`enroll KEY` makes a Secure Enclave signing key requiring user presence and saves
its opaque wrapped representation, with file mode 0600. Keep this file on the
creating Mac. The public key is the only key material needed on comte.
`sign KEY PAYLOAD allow` uses a fresh LocalAuthentication context. macOS may offer
Touch ID or the permitted account-password fallback; this probe does not require
biometrics exclusively. It displays the challenge and puts the decision in the
signed message. A deny decision is also signed.

An example exchange uses `pending` on comte, `sign` on kiwi, and `submit` on comte.
The transport for those files can be SSH. No SSH session, credential forwarding
or remote host access is implemented or started by this prototype.

## Real Authorization Services integration

Installation is a separate, privileged experiment. First inspect and sign the
built plug-in and broker with the development identity chosen for the test host.
Install only into an isolated test machine/account environment with a recovery
path. Loading and completing the custom right through both automatic policy and
a Secure Enclave signature from kiwi were verified on comte (macOS 26.5.1);
see NOTES.md for remaining checks.

The intended installation consists of:

- The root-owned, non-user-writable plug-in bundle at
  `/Library/Security/SecurityAgentPlugins/PortholeAuthorizationPrototype.bundle`.
- A root-owned broker executable and enrolled public key in a dedicated directory
  under `/Library/Application Support/PortholeAuthorizationPrototype`.
- A root-owned mode-0755 `/var/run/porthole-auth-prototype` directory. The broker
  listens at `broker.sock`; the socket permits transport clients to submit signed
  decisions. Only a root peer may originate a real mechanism request.
- The single right `work.flotilla.prototype.remote-approval`, using `right.plist`.
  Do not overwrite an existing definition if one is already installed.

Start the broker as root with `serve SOCKET PUBKEY`, then run `request` as the
ordinary user. The plug-in connects only to the fixed socket above, verifies
its peer is root, and asks the broker for a decision. The application reports
`AuthorizationCopyRights`'s actual result. Restart the broker with `--automatic`
to exercise the standing-policy path. Denial, timeout and cancellation should
be exercised too; successful signatures alone do not prove OS integration.

`install-comte.sh` stages the agreed comte experiment: it refuses existing
prototype paths/rights, verifies the signing team, installs the custom right and
root broker, runs the automatic path as robert, then leaves the broker in human
mode. It requires an administrator to invoke it explicitly. The installer also
compares taskport policy before and after. Rollback removes only this named
right and these prototype files after
stopping the broker and finishing outstanding authorization calls. A loaded
plug-in can remain in its host process until that process exits.

## Boundaries and evidence

The request describes the custom right and host, not the original application's
identity or a target PID. The broker authenticates the mechanism's root peer,
not the original Authorization Services client. Its socket API exposes pending
test challenges to local users and has no production resource limits. These
choices are acceptable only because the custom right performs no operation.

The C mechanism uses an asynchronous worker and interrupts socket I/O when
cancelled; actual host callback/reentrancy and unload behavior need a real
installation test. A 60-second human-decision deadline is intentionally short.

Keep outcomes in NOTES.md. Delete or replace this prototype once the API and
policy questions are answered; do not make it a Porthole bundle dependency.
