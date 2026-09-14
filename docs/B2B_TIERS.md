# B2B tiers and commission pricing

Frontend follow-up (2026-09-14): the sibling ShoponTravels frontend now implements commission settings and client tier controls inside API Management. Migration 0023 adds managed-client eligibility and identity linking. See [API Management](API_MANAGEMENT.md). Earlier statements below describe the original backend-only increment; frontend booking-engine integration and settlement remain separate.

Implemented locally on 2026-09-14. Migrations `0021_b2b_tiers.sql` and `0022_tier_share_policy.sql` are required. This feature does not enable supplier execution, API Management eligibility, payments, wallets or settlements.

## Business rule

Existing audience/Specific Agent/route/airline markup resolution stays unchanged. Select the lowest equivalent **original supplier fare** first, then calculate the existing marked-up selling fare. That selling fare is called **gross** in this tier API. Do not confuse it with supplier base+tax “gross” or legacy Search summary `minPrice` fields.

Default shares (Admin-configurable):

| B2B tier | Agent share of markup | Platform share of markup |
| --- | ---: | ---: |
| basic | 60% | 40% |
| professional | 80% | 20% |
| enterprise | 100% | 0% |

Per passenger: `commission = markup pool × tier share`; `payable = gross − commission`. With supplier total 10,000 and markup 7%, gross is 10,700. Basic commission/payable are 420/10,280; Professional 560/10,140; Enterprise 700/10,000. This applies to fixed markup too. The percentages are shares of the **resolved markup**, not an additional percentage of gross or a second markup rule.

Exact decimal calculations use two-decimal half-up rounding. The distributable pool is rounded gross minus the supplier passenger total rounded to the same precision. Commission is rounded per individual passenger, payable is the remainder, then counts are multiplied and passenger types aggregated. At sub-cent boundaries the effective platform/agent percentage may differ because a currency cent cannot be split. Zero markup gives zero commission. Base, taxes, AIT, supplier discounts/references and gross projection are not rewritten by tier calculation.

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

Existing supplier-compatible flight bodies and `totalPrice` fields continue to represent the existing gross selling fare. **Use the new pricing API's `payable` for the tier price display.** Existing Search price filters summarize the legacy selling fare; frontend payable filters must use tier pricing data. `discountPrice` is the existing supplier-compatible calculation, not the tier commission.

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

Search and RePrice each capture the authenticated tier and the current configured share in the same database read at request entry and persist their pricing atomically with their existing snapshots. Tier/markup changes affect subsequent quotes, never mutate existing snapshots or accepted bookings. A booking always uses the priceCodeRef it accepted. Database triggers reject snapshot updates. Supplier request payloads never include tier commission data.

Swagger exposes these endpoints under **B2B tiers**. The separate `ShoponTravels` frontend and its sidebar/pages have not been changed in this backend increment.

## Verification

63 regular unit/foundation/fixture tests passed. Full disposable PostgreSQL integration passed (40.23 seconds), including tier assignment permissions, invalid tier rejection, audit, owner isolation, batch validation, historical-snapshot refusal, immutable snapshots, tier-only RePrice changes and booking pricing stability after tier changes. All-target Clippy with warnings denied, formatting and diff checks passed. No real supplier calls, working database migration or production deployment were performed.

Configurable-share follow-up: 64 regular tests and the full disposable database suite passed (39.74 seconds). Coverage includes persisted custom shares in Search/RePrice, share-only price-change detection, immediate existing-token updates, ordinary Admin control, machine-token rejection, invalid percentages/order, concurrent stale-write rejection, immutable booking pricing and policy audit. All-target Clippy and formatting checks passed. Migration 0022 has only been applied to the disposable test database.
