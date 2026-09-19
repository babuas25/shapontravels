# Ticket-management API

These endpoints send an agency's refund, reissue or void request to **Shapon Travels staff**. They do not send requests to FirstTrip, Triplover, Takeoff or any other supplier. Staff use the existing dashboard queue to review, quote, assign and manually complete the work.

Use the Rust API base URL (locally `http://127.0.0.1:18081`), **not** the Next.js dashboard origin. The dashboard has similarly named session-authenticated routes.

## Authentication and permissions

Exchange existing client credentials at `POST /auth/token` and send `Authorization: Bearer <stm_…>` on every request. Admin sessions, portal search tokens and caller-supplied user/agency identities are not accepted.

Grant `ticket-management:read` for eligibility, list and detail; grant `ticket-management:write` for creation and quotation decisions. Both are available in API Management. Migration 0052 permits these grants but does not add them to existing clients. A client must have an existing wallet-owner link; bookings must belong to that exact API client and the linked wallet owner. Sharing a wallet does not share booking access. Managed clients also require active Enterprise API access and an active canonical owner.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| GET | `/api/ticket-management/availability?bookingReference=STR…` | Passenger availability, zero-based journey indices and available actions |
| POST | `/api/ticket-management` | Submit a refund, reissue or void request to staff |
| GET | `/api/ticket-management` | Latest scoped requests, newest first |
| GET | `/api/ticket-management/{requestId}` | Status, quotations, decisions and customer-safe history |
| POST | `/api/ticket-management/{requestId}/decision` | Accept or reject a specific quotation |

List filters: `bookingReference`, `action`, `requestType`, `status`, `limit` (1–100, default 50). This is a bounded recent list, not a full-history export. Retrieve a known request directly by its UUID. Responses use the JSON shapes in `/openapi.json`; they are not supplier `item1` envelopes.

## Submit a reissue request

```json
{
  "bookingReference": "STRABC123XYZ789",
  "requestId": "b37a4189-a4ce-4d91-bc0c-e04156f0d71a",
  "action": "reissue",
  "requestType": "voluntary",
  "passengerIndexes": [0],
  "routeIndexes": [0],
  "reissuePreferences": [
    {
      "routeIndex": 0,
      "departureDate": "2026-12-15",
      "preferredFlight": "BG morning flight"
    }
  ],
  "note": "Please quote this date change."
}
```

Generate a new UUID for each new request. Retry an uncertain submission with the **same UUID and unchanged body**. Initial creation returns 201; an exact replay returns 200 with `replay: true`. Reusing the UUID with different content returns 409.

`action`: `refund`, `reissue`, `void`. `requestType`: `voluntary` or `involuntary`. Select at least one passenger and one route, without duplicates. These are indices from the saved booking/availability response, not passenger names or new ticket numbers.

Reissue requires exactly one preference per selected route. Dates use `YYYY-MM-DD`, cannot be before today's Bangladesh date, and must be nondecreasing in route order. `preferredFlight` is optional (80 characters); the free-text note is optional (1,000 characters). All preferences plus notes must fit the 2,000-character staff note. Preferences are persisted as structured snapshot data and also rendered in the existing staff request note. They express requested travel, not confirmed airline availability. The selected original route endpoints remain unchanged; requests for a different origin/destination are not modeled by this contract.

For refund or void, omit `reissuePreferences` (or send `[]`). The backend reads ticket numbers, identity and financial entitlement from saved booking/issuance records. Requests require verified issued tickets with a captured wallet charge and supported per-passenger entitlement. Imported, unpaid or ambiguous historical tickets cannot invent refundable credit. Another active request on the same passenger is rejected. VOID intake follows the existing issue-date cutoff at 23:30 Bangladesh time; staff must still verify actual airline eligibility.

Example creation result:

```json
{
  "ok": true,
  "requestId": "b37a4189-a4ce-4d91-bc0c-e04156f0d71a",
  "publicRef": "TMRE000000000001",
  "status": "requested",
  "outcome": null,
  "version": 1
}
```

Submission does not change balances or execute a supplier operation. The request appears in the same admin queue and uses the existing notification outbox policy. Notification transport remains governed by its separate enablement settings.

## Quotation decision

Read request detail, display the active quotation and its deadline, then submit:

```json
{
  "requestKey": "034fb5ec-4c1d-4bc0-8b57-ef369bab2b24",
  "expectedVersion": 3,
  "quoteId": "885ea332-fa92-4595-a19d-bf69904868c2",
  "decision": "approved",
  "note": "Agency accepts the quotation."
}
```

Use `rejected` to decline. `requestKey` is a new UUID per decision; reuse it only for an identical retry. `quoteId` and `expectedVersion` come from current detail. Changed/expired quotations, stale versions and insufficient wallet funds are rejected. An approved **debit** quotation reserves funds once; a credit quotation does not credit the wallet immediately. Staff completion performs settlement through the existing wallet workflow.

Amounts in quotations are exact **minor units**, alongside `currency`: `100` means BDT 1.00 or USD 1.00. Do not submit a client-calculated amount; acceptance refers to the stored quotation.

`approved` with a null outcome means the agency accepted and staff completion is pending. Completion sets `terminalOutcome` to `refunded`, `reissued` or `voided`. Other terminal outcomes include `staff-rejected`, `customer-rejected` and `confirmation-expired`. After reissue, detail includes `reissuedTickets` with passenger index and previous/new ticket numbers. An idempotent response represents the original operation result; use GET detail for current status.

## Errors and operational boundaries

Errors are `{"error":"CODE"}`. Common responses: 401 invalid/expired credentials, 403 missing permission or inactive/unlinked owner, 404 unknown/foreign booking or request, 409 duplicate active request, conflicting idempotency key, closed void window, stale quote/version or wallet conflict, 422 invalid selection or preferences, 429 rate limit and 503 unavailable storage/rollout.

Only customer request/decision operations are exposed. Clients cannot publish quotes, assign staff, release holds or mark requests complete. Internal notes, wallet operation IDs, staff assignment details and entitlement funding are not returned. No supplier adapters are called by these routes. No new automatic email/SMS sending is enabled by this feature.
