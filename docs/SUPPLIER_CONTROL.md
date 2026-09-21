# Supplier Control

Released to development and production on 2026-09-21 with backend `06d71a0`
and frontend `8fd2fd5`. The canonical Super Admin sidebar links
`/dashboard/supplier-control`. FirstTrip, TripLover and TakeOff each have an
independent Search switch; any subset can be active. All-off explicitly shows
that new searches are unavailable. Equal-fare priority remains
FirstTrip → TripLover → TakeOff after lowest-fare selection.

The page uses Rust supplier connections, not the legacy frontend
`active_supplier` setting. Only a current Super Admin can read or write through
Next `/api/supplier-control` and the canonical identity bridge. Browser writes
require same-origin requests, bounded/strict inputs and a current row version.
Supplier credentials and URLs are never returned. `configured` means that the
backend has an adapter and currency, not that a live authentication probe passed.

## Rust contract

`POST /admin/supplier-search-control` accepts:

```json
{"action":"list"}
```

or one row change:

```json
{"action":"set","supplier":"takeoff","search_enabled":true,"expected_version":2}
```

Response: `{ "suppliers": [...] }`, containing exactly three records with
`id`, `search_enabled`, `configured`, `timeout_seconds` and `version`.

- The canonical business allowlist and handler both require Super Admin authority.
  Writes recheck authority within the identity transaction barrier.
- Each change atomically updates Search enablement, row version and its audit
  event. Timeout, servicing, booking and ticketing controls are preserved.
- Disabling increments the availability epoch so prior unbooked offers cannot
  be reused after re-enabling. Existing booked records are not modified.
- Stale versions return `409 CONFIGURATION_CHANGED`; missing configured adapters
  or currency block enabling with `422 SUPPLIER_NOT_CONFIGURED`. Disabling remains
  possible when configuration is missing.
- The frontend waits for confirmation before changing its switch state. Errors
  disable further writes until a refresh. Unknown responses are never retried
  automatically; refresh reads back persisted state.
- The endpoint does not authenticate to suppliers or create bookings/tickets.
  Active suppliers still follow existing search quotas, pricing validation and
  configured response deadlines.

No database migration is required. Release the backend before the frontend using
the deployment runbook. No supplier is enabled just by publishing this feature.

## Verification

- Disposable PostgreSQL canonical integration covers all eight supplier subsets,
  active Search snapshots, ordinary-admin/staff/customer denials, stale/invalid
  writes, missing configuration, audit events, offer invalidation and preserved
  timeout/servicing/booking/ticketing controls.
- Frontend boundary checks cover role restrictions, sidebar visibility, CSRF,
  version forwarding, strict records and the canonical middleware policy.
- Browser fixtures exercise the actual React panel with 0/1/2/3 enabled,
  successful writes, stale state, unknown-response recovery, missing setup and
  desktop/mobile layouts. They do not alter live settings.
- Rust library tests, all-target Clippy, TypeScript and changed-file ESLint pass.
  The canonical regression tests are included in each repository's CI checks.
