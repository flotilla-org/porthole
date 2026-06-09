# Agent-permission authority is deferred; porthole verifies tokens, it does not own authority

Porthole's current agent-permission layer (identities, token mint/grant via the
`porthole agents` CLI) collapses the operator and the agent into a single
local-user principal, so an agent can grant itself any capability. The route-guard
*enforcement* is real; the *authority* behind it is not. Rather than build an
authority model inside porthole — the phase-3 notification-approval UI, richer
operator separation — we defer it. The problem is bigger than desktop driving
(the same authority question covers talking to APIs and other agent capabilities),
it spirals easily, and it deserves a dedicated research pass over existing
capability/authorization models and human-in-the-loop approval components before
we commit to one.

The intended end shape: porthole is a **token consumer/verifier** (JWT-esque — it
validates a scoped token the caller presents and enforces it at the route guard, a
policy *enforcement* point), and the **platform helper is a pluggable auth
surface** — the place a human approves a request, the macOS helper today, a phone
or other device later — not the authority itself. The authority/decision plane
lives outside porthole and is expected to generalise beyond it.

## Consequences

- Near-term the `porthole agents` CLI path is a **local-trust dev placeholder**,
  documented as such, not a security boundary. Enforcement may be toggled off
  entirely if it impedes the jackstay work — it is not load-bearing yet.
- **Do not** build the notification-based approval UI or extend enforcement
  coverage until the research pass picks a model. Building more on the self-grant
  foundation just creates more to unwind.
- When a model is chosen, expect the identity / authority / policy-decision pieces
  to extract into a general capability service; porthole keeps only the
  enforcement point and a token-verification dependency.
- The roadmap's phase-2.5 "operator authority" and phase-3 "approval notification"
  items are deferred under this ADR rather than open work.
