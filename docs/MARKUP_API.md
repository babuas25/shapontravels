# Admin markup rule API

Authorize `admin_session` in Swagger. Both Admin and Super Admin may manage rules. Machine tokens cannot read or mutate these administrative endpoints.

| Operation | Endpoint |
|---|---|
| Create inactive draft | `POST /admin/markup-rules` |
| List rules | `GET /admin/markup-rules?limit=50&offset=0` |
| Read rule/current version | `GET /admin/markup-rules/{id}` |
| Edit complete rule definition | `PUT /admin/markup-rules/{id}` |
| Activate/deactivate | `PUT /admin/markup-rules/{id}/status` |

## Swagger walkthrough

Create this rule using POST:

```json
{
  "name": "B2B default",
  "audience": "b2b",
  "agent_id": null,
  "airline": null,
  "origin": null,
  "destination": null,
  "kind": "fixed",
  "amount": "500",
  "currency": "BDT"
}
```

The response is 201 with `id`, `version: 1` and `active: false`. Copy its ID into the status endpoint and send:

```json
{"expected_version": 1, "active": true}
```

The 200 response has `active: true` and a new version. Use the latest version from GET or a mutation response for subsequent edits/status changes. For deactivation use `active: false`. An edit takes the same complete fields as creation plus `expected_version`; active rules may be edited, subject to the duplicate constraint. Each successful edit/status change creates a new immutable version and audit event.

To test duplicates, create another draft with the same audience/agent/airline/route, then try to activate it while the first remains active. Expect 409 with `ACTIVE_MARKUP_SCOPE_CONFLICT`. The existing active rule remains unchanged. Deactivate the first or edit it by ID; activation does not replace it automatically. Concurrent activation and edits are protected by a PostgreSQL unique index (including null/all scopes) and row locks. A stale version returns 409 `RULE_VERSION_CONFLICT`.

## Fields and scope

- `audience`: `b2b`, `b2c`, or `specific_agent`. Only `specific_agent` takes an `agent_id`; use the same trusted agent UUID assigned to the relevant API client(s).
- `airline: null`: all airlines; otherwise an uppercase two-character carrier code.
- Both route fields null: all routes. Otherwise provide both uppercase three-letter airport codes, e.g. DAC/SIN. Route matching against return/multicity/mixed-airline offers is a separate pending decision; storing a scope does not choose that algorithm.
- `kind`: `fixed` or `percentage`. `amount` is an exact nonnegative decimal STRING (up to 60 input characters), avoiding floating-point parsing. Use `"0"` only when intentionally configuring a zero-markup rule.
- `currency`: explicit uppercase three-letter pricing currency. It is not proof of supplier Search currency. Currency is not a separate duplicate scope: two active rules cannot bypass the approved scope constraint by using different currencies.
- New drafts always start inactive; arbitrary `active`, owner, version or other unknown create fields are rejected. Identical inactive drafts are allowed; the approved conflict policy applies to active rules.
- List pages accept limits 1–100, default 50, and a nonnegative offset. No delete endpoint is provided; deactivate preserves audit/history.

Active rules are used by public Search. Complex offers support All airline / All route rules only when the applicable audience/agent and currency candidate set contains no scoped rule; other mapping and price-coverage checks remain in place. See `SEARCH_API.md`. Activating a rule does not call a supplier or issue a ticket.

## Verification

Local PostgreSQL integration tests cover draft creation, listing/reading, exact decimal storage, validation, machine-token rejection, optimistic concurrency, duplicate null scopes, active editing, deactivate/reactivate replacement, conflicting active scope edits, immutable versions and audit. Unit/fixture tests exercise the existing pricing engine. Tests use disposable databases; no test rule was inserted into the user's working development database.
