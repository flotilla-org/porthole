# Capture Transfer FD Auth Design

Date: 2026-05-18
Status: approved follow-on to `2026-05-17-porthole-agent-permissions-design.md`

## Context

Agent permissions now protect surface capture-session creation, but the raw
capture-transfer Unix socket still accepts frame requests from any same-user
process that knows the socket path and session id. That was acceptable while the
transport was a prototype; it is now the remaining Phase 2.5 enforcement gap.

The approved agent-permissions spec defines `record` as covering
capture-transfer consumption. This slice applies that decision to the fd
channel without replacing the current newline-JSON plus `SCM_RIGHTS` transport.

## Goals

- Require a bearer-token authorization handshake before a raw fd-channel
  connection can consume frames from a protected surface capture session.
- Bind protected surface capture sessions to the agent identity that created
  them.
- Keep synthetic capture sessions unauthenticated because they do not touch the
  real desktop.
- Preserve the existing frame request/release messages after authorization.
- Keep already-open connections stable; token revocation affects new fd-channel
  connections.

## Non-Goals

- No daemon-mediated HTTP frame read path.
- No per-frame bearer token in every request.
- No hard same-user malicious-code sandbox claim.
- No helper/operator authority changes.
- No synthetic-session protection in this slice.

## Protocol

Add a client-to-daemon message:

```json
{ "op": "authorize", "session_id": "capture-...", "bearer_token": "pta_agent..." }
```

The first message on a connection may be `authorize`. Once the daemon accepts
it, subsequent frame acquisition and release messages use the existing protocol.

For compatibility with synthetic test and developer sessions, the daemon may
also accept a first frame request for sessions that have no owner agent id.
Protected sessions reject any frame acquisition until `authorize` succeeds.

The daemon closes the connection on failed authorization or unauthorized frame
acquisition. The fd channel intentionally does not grow a rich error-response
vocabulary in this slice; existing clients already treat connection failure as a
transport error.

## Session Ownership

`CaptureSession` stores `owner_agent_id: Option<AgentId>`.

- Surface capture sessions pass the already-authorized route execution's
  `agent_id` into `create_surface_session`.
- Synthetic sessions store `None`.
- Frame acquisition checks the requested `session_id` against the connection's
  authorized agent id:
  - no owner: allowed
  - owner matches authorized agent: allowed
  - owner exists and no/mismatched authorization: reject

Release remains lease-id based on the connection-local lease table. A client
cannot release a lease it did not acquire on that connection.

## Client Behavior

`capture-transfer::daemon::SessionInfo` gains an optional bearer token. The
daemon consumer sends `authorize` immediately after connecting when the token is
present. `porthole record` passes the CLI client's agent token into the
`RecordSession` and then into `SessionInfo`, so recording continues to work with
the same token used to create the surface capture session.

FFI/synthetic helpers can omit the token for synthetic sessions.

## Testing

- `capture-transfer` request-message round trip covers `authorize`.
- `portholed` fd-channel tests prove:
  - a protected session rejects frame requests before authorization
  - a protected session rejects the wrong token
  - a protected session serves frames after a matching authorization
  - synthetic sessions still serve frames without authorization
- `porthole record` unit tests prove the bearer token from the HTTP client is
  carried into the fd-channel consumer.

