# B2B tiers and commission pricing

Frontend follow-up (2026-09-14): the sibling ShoponTravels frontend now implements commission settings and client tier controls inside API Management. Migration 0023 adds managed-client eligibility and identity linking. See [API Management](API_MANAGEMENT.md). Earlier statements below describe the original backend-only increment; frontend booking-engine integration and settlement remain separate.

Backend released on 2026-09-14 as `44fc47d`, including migrations `0021_b2b_tiers.sql` and `0022_tier_share_policy.sql`; local working and production migration readiness are verified. See [release evidence](evidence/API_MANAGEMENT_RELEASE_2026-09-14.md). Historical local-only verification notes below describe the implementation stage. This feature does not enable supplier execution, API Management eligibility, payments, wallets or settlements.

## Business rule

Final user clarification on 17 September 2026: **gross = original base fare + taxes**. First calculate **supplier total + resolved markup**. The available discount is gross minus this marked-up supplier total. Tier percentages share that available discount, not the markup itself. This supersedes earlier same-day interpretations recorded in the evidence files.

Per passenger:

```text
marked_up_supplier = round_half_up(supplier original totalPrice + resolved markup, 2)
available_discount = gross − marked_up_supplier
agent_discount = round_half_up(available_discount × tier share / 100, 2)
agent_payable = gross − agent_discount
```

Use the original supplier total as the percentage markup basis (including original AIT once); do not reconstruct it from base/taxes or add AIT again. Fixed markup is per individual passenger. Multiply rounded passenger amounts by counts. At 100% share, agent payable equals the marked-up supplier total exactly. Increasing markup increases that payable and reduces the available discount.

Current saved local shares **55% / 85% / 100%** were explicitly retained by the user. Migration defaults remain 60% / 80% / 100%; the saved Admin-configured policy is authoritative.

For base 4,624, taxes 1,125, supplier total 5,263.48 and markup 1%: gross 5,749.00; marked-up supplier total 5,316.11; available discount 432.89.

| Tier/share | Agent discount | Agent payable |
| --- | ---: | ---: |
| Basic 55% | 238.09 | 5,510.91 |
| Professional 85% | 367.96 | 5,381.04 |
| Enterprise 100% | 432.89 | 5,316.11 |

The B2B fare table and RePrice review label the deduction **Discount**. For wire and historical snapshot compatibility, the existing `commission` field carries this gross-to-payable discount; it does not represent the supplier markup. The invariant remains `gross = commission + payable`. Discount may be negative when supplier plus markup exceeds gross; preserve the signed result without clamping or dropping otherwise valid offers. Wallet payable remains a positive monetary amount, with only its reconciliation adjustment accepting a signed value.

Cards show **Gross fare** as the large primary amount. The arrow reveals **Agent fare** for B2B or **Supplier fare** for staff; secondary amounts start hidden. No tier names or percentages appear on the card. Staff retain published gross and private supplier cost. Separate AIT is shown as evidence, excluded from gross. B2C keeps its existing projection. Original supplier responses and historical accepted snapshots remain immutable. Search/RePrice projected totals continue to carry published gross.

All B2B clients, including existing ones, start at Basic when migration 0021 is applied. B2C receives no tier commission (`tier: null`, share 0, payable equals gross). Historical quote/booking records remain untouched: their missing snapshots return 409 `PRICING_SNAPSHOT_UNAVAILABLE`; do not calculate historical commission from today's tier or markup.

## Admin share configuration

Both human Admin and Superadmin can read and update the global shares. B2B machine tokens cannot access these endpoints. Client tier assignment remains Superadmin-only.

`GET /admin/tier-policy` returns the current policy, initially:

```json
{"version":1,"basic":60,"professional":80,"enterprise":100}
```

Save all three shares together with the version returned by GET:

```http
PUT /admin/tier-policy
Authorization: Bearer <human-admin-session>
Content-Type: application/json

{"expected_version":1,"basic":50,"professional":75,"enterprise":95}
```

The response includes the saved percentages and incremented `version`. Values are whole percentages from 0 to 100, with `basic <= professional <= enterprise`; invalid values/order return 400 `INVALID_TIER_POLICY`, and fractional or extra properties are rejected. Equal percentages and zero are allowed. A stale version returns 409 `TIER_POLICY_VERSION_CONFLICT`; reload before saving again. Changes are atomic and audited with the actor and previous/new policy.

Shares apply globally to all clients in each tier on subsequent Search/RePrice requests, including with existing access tokens, without restart or redeployment. `/auth/me` exposes `commission_share_percent`; B2C always has zero share. Existing quote and booking snapshots retain their saved percentage, commission and payable. A changed share alone can require updated RePrice acceptance even when tier and gross are unchanged.

Frontend can use these endpoints for a **Tier Commission Settings** form with three percentage inputs and a Save button. That frontend form is not implemented in this backend increment.

## Superadmin control

Existing `POST /admin/clients` provisions a Basic B2B client. Existing client updates do not reset its tier. Use the dedicated endpoint to assign another tier:

```http
PUT /admin/clients/{clientUuid}/tier
Authorization: Bearer <human-superadmin-session>
Content-Type: application/json

{"tier":"enterprise"}
```

Response:

```json
{"clientId":"<clientUuid>","tier":"enterprise","commissionSharePercent":100}
```

`GET /admin/clients/{clientUuid}/tier` is available to authenticated human administrators. Only a Superadmin can PUT; ordinary Admin receives 403, machine tokens receive 401. Unknown/B2C clients return 404. Unknown tier values or extra JSON properties are rejected. Changes serialize on the client row and record previous/new tiers in the audit log.

`GET /auth/me` includes `tier` from current trusted client configuration, including for existing access tokens. API clients cannot supply their tier in Search/RePrice or change it through commercial endpoints. API access permissions and rate limits remain separate settings; this increment does not introduce Enterprise-only API access.

## Frontend integration

New B2B supplier-compatible flight bodies and `totalPrice` fields represent published gross (base + taxes). **Use the new pricing API's `payable` for the tier price display.** Search price filters summarize projected gross; frontend payable filters use tier pricing data. `discountPrice` is the existing supplier-compatible calculation, not the tier commission.

1. After Search, collect returned platform `itemCodeRef` UUIDs and fetch pricing in batches of up to 100 distinct IDs:

   ```http
   POST /api/pricing/offers
   Authorization: Bearer <machine-token>
   Content-Type: application/json

   {"offer_ids":["<offerUuid>"]}
   ```

   The response is an object keyed by offer UUID. All IDs must belong to the caller. Unknown/foreign IDs return 404 for the whole batch, never partial private data. Empty, duplicate or oversized batches return 422 `INVALID_PRICING_BATCH`.

2. Single-record pricing uses `GET /api/pricing/offer/{offerUuid}` or `GET /api/pricing/reprice/{priceCodeRef}`. Both require `search:read` permission.
3. After RePrice, retrieve the **new** priceCodeRef's pricing, display gross/commission/payable, then explicitly accept that revision. A payable change marks `isPriceChanged` even when gross is unchanged.
4. After Book, use `GET /api/pricing/booking/{bookingUuid}` with `booking` permission. It returns the booking's accepted RePrice snapshot, including after ticket Issue/report retrieval. Gross supplier-compatible receipts and reports remain unchanged. Existing STR lookup supplies the booking UUID when needed.

Example single pricing response (all money values are exact decimal strings):

```json
{
  "version":1,
  "tier":"basic",
  "commissionSharePercent":60,
  "currency":"BDT",
  "gross":"6000.00",
  "commission":"60.00",
  "payable":"5940.00",
  "passengers":{
    "ADT":{"count":1,"gross":"6000.00","commission":"60.00","payable":"5940.00"}
  }
}
```

Passenger amounts are per individual; top-level amounts include counts. The response does not expose supplier account credentials or private passenger information. A stored pricing response is not proof that a quote is still valid or a booking has succeeded; existing expiry, acceptance and booking-state checks still apply.

Search and RePrice each capture the authenticated tier and the current configured share in the same database read at request entry and persist their pricing atomically with their existing snapshots. Tier/share changes and this gross correction affect subsequent quotes, never mutate existing snapshots or accepted bookings. A booking always uses the priceCodeRef it accepted. Database triggers reject snapshot updates. Supplier request payloads never include tier commission data.

Swagger exposes these endpoints under **B2B tiers**. The separate `ShoponTravels` frontend and its sidebar/pages have not been changed in this backend increment.

## Verification

63 regular unit/foundation/fixture tests passed. Full disposable PostgreSQL integration passed (40.23 seconds), including tier assignment permissions, invalid tier rejection, audit, owner isolation, batch validation, historical-snapshot refusal, immutable snapshots, tier-only RePrice changes and booking pricing stability after tier changes. All-target Clippy with warnings denied, formatting and diff checks passed. No real supplier calls, working database migration or production deployment were performed.

Configurable-share follow-up: 64 regular tests and the full disposable database suite passed (39.74 seconds). Coverage includes persisted custom shares in Search/RePrice, share-only price-change detection, immediate existing-token updates, ordinary Admin control, machine-token rejection, invalid percentages/order, concurrent stale-write rejection, immutable booking pricing and policy audit. All-target Clippy and formatting checks passed. Migration 0022 has only been applied to the disposable test database.
