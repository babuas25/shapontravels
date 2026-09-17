# Portal Book & Hold

The portal supports active B2B users booking for themselves and Super Admin booking on behalf of an active B2B owner. It reuses the native Search, RePrice, explicit price acceptance and idempotent Book state machine. It never grants ticketing, cancellation or external API credentials.

## Authority and ownership

The Next bridge authenticates fresh Clerk metadata on every action. Browser role previews, owner IDs, membership tiers and supplier references are not authority. All routes below require the existing Super Admin `sta_` bridge token; a B2B user's attested role is carried separately.

`identity` is `{draft_id, owner_external_user_id, actor:{external_user_id,role:"superadmin"|"b2b"}}`. B2B actors must equal the owner and may prepare only offers from their own linked, active B2B client's search. Super Admin may prepare only their own staff search offers. Every read, acceptance and submission checks the immutable creator, owner and client associations. An owner can read a reserved on-behalf receipt, but cannot operate another creator's private draft.

Only Super Admin can list assignees or provision a missing linked B2B client. Existing account tier, suspension and permissions remain authoritative. Self-booking does not provision clients.

## Routes

| POST route | Input | Result |
| --- | --- | --- |
| `/admin/portal-holds/prepare` | identity, source_offer_id, complete segment_code_refs, trusted owner_display | Owner-scoped repriced draft and submission gate |
| `/admin/portal-holds/read` | identity | Stored draft and durable booking outcome |
| `/admin/portal-holds/accept` | identity, price_id, exact pricing snapshot | Native latest-price acceptance |
| `/admin/portal-holds/submit` | identity, passengers, customer contact | Native Hold Book followed by stored outcome |
| `/admin/portal-holds/receipt` | reader, draft_id, optional refresh (default false) | Reserved receipt; explicit refresh reads supplier PNR after owner authorization |
| `/admin/portal-holds/recent` | reader, optional before booking UUID | Twenty scoped records and next cursor |
| `/admin/portal-holds/dashboard` | reader, validated table query, trusted creator IDs | Original Ticket table rows, database filtering and pagination |

`reader` is `{external_user_id,role:"superadmin"|"b2b"}`, supplied by the authenticated bridge. History exposes only reserved bookings. Foreign-owner cursors and receipts reveal no records. Supplier cost is staff-only. Creator and owner equality distinguish self-bookings from on-behalf bookings in the dashboard.

## Booking behavior

Migration `0027_portal_hold_drafts.sql` stores immutable owner/creator associations. Preparation copies the selected offer into an owner-scoped search, retaining supplier provenance, complete directions and original expiry. Native RePrice applies the owner's current tier and commission. Switching owner or fare requires a new draft and price review. Both Search and RePrice must support Hold; instant-ticketing fares are rejected.

Submission requires explicit acceptance of the latest price and checks account/tier, supplier controls, expiry, selected passenger counts, ages, titles and documents. Portal passenger rules match the existing checkout: surname-only bookings are allowed, domestic passports are omitted, and international passport format/expiry is validated from the stored route. See the frontend `docs/PASSENGER_VALIDATION_PARITY.md`.

The draft UUID is the stable client-scoped idempotency key. Owner, creator audit and customer contact are stored with the dispatch reservation. Customer contact is included in the request hash but not sent to suppliers; the existing fixed agency contact is used. Concurrent clicks dispatch once. A pending or ambiguous outcome is read back and never automatically redispatched. Page load and receipt reload read the database only.

Saved passenger operations in a Super Admin hold context resolve the owner from the authorized draft. B2B self-booking uses the normal own-profile scope. Cross-owner profiles are inaccessible, new profiles retain the actual creator audit, and existing REF generation is preserved.

## Explicit status/deadline refresh

The existing receipt route accepts `refresh:true`. It authorizes Super Admin or the booking's B2B owner, requires an active linked Rust B2B client, and calls the same `booking::reconcile_owned` helper as `/api/bookings/{id}/reconcile`. Supplier servicing must be enabled. Stored references are resolved in Rust; the browser provides only the draft UUID. No new Rust endpoint or machine credential is needed for the portal.

The browser's **Refresh Status / Deadline** button posts `{action:"refresh",draftId}` through the same-origin Next bridge. Fresh Clerk identity supplies the reader. It sends one PNR read and then returns the saved receipt. Automatic Search/RePrice/Book/NewTicket and ordinary receipt loads do not call PNR. Refresh never dispatches Book, Issue or Cancel.

`booking.details.pnrObservation` is the newest verified observation by request timestamp: `status`, `lastTicketTime`, `checkedAt`, and `manualResolutionRequired`. It contains no raw supplier payload. `booking.details.ticketingTimeLimit` uses that observation's valid deadline, even when null; otherwise it uses the saved Book deadline. A verified absent/invalid deadline does not fall back to an older Book value. The documented PNR format is `MM/dd/yyyy HH:mm:ss`; no timezone or countdown is inferred. The check timestamp itself is an explicit UTC instant.

Failed/mismatched reads return an error and preserve the last verified observation and its timestamp. The UI retains prior details and labels the failed refresh. A verified terminal/conflicting status displays manual review rather than promoting the local booking to issued/cancelled or showing it as ready to issue. Migration **0028** is required before serving this build; this addition requires no further migration.

## Verification and limits — 15 September 2026

- Actual signed-in frontend → Rust → Triplover UAT completed one adult, one-way domestic Hold: Arif Hasan, PNR `0A4OXY`, no ticket issue. A read-only supplier PNR check returned `Booked`. See [the UAT review](evidence/PORTAL_HOLD_UAT_REVIEW_2026-09-15.md).
- A subsequent international Bangkok return Hold completed for Nadia Rahman, PNR `0A4OYD`, payable BDT 41,932; supplier PNR status is `Booked`. The actual frontend form, price acceptance, Hold, receipt reload and Ticket list passed. [Alternate-route evidence](evidence/PORTAL_ALTERNATE_ROUTES_UAT_2026-09-15.md) records the supplier cabin mismatch, list route formatting and remaining mixed-passenger coverage.
- B2B self-booking passed real Next handlers → Rust HTTP → isolated PostgreSQL, with simulated Clerk and supplier responses. Coverage includes own search/price/passengers/receipt, cross-owner and source-offer rejection, role demotion, no self-provisioning, creator labels, concurrent retry and uncertain outcomes. This does not establish a live signed-in B2B UAT Hold.
- Extended international UAT searches to SIN and CCU on 30 September failed with supplier-unavailable errors. Direct DAC → SIN Search returned `503 ALL_SUPPLIERS_FAILED` after 120.3 seconds. See [the continuation record](evidence/PORTAL_HOLD_CONTINUATION_2026-09-15.md).
- The isolated verification example uses simulated suppliers. The separate UAT server enables only the explicit Triplover UAT Hold transport; production booking and ticket issue remain prohibited for this work.
- Explicit receipt PNR refresh is connected locally and verified with simulated supplier responses through real Next/Rust/PostgreSQL. The new UI action has not been exercised against a live supplier. Legacy history, completed multicity/mixed-passenger Holds and real B2B self-service UAT remain outstanding. Draft-linked offers are retained for provenance; ordinary native administration/reconciliation remains available.


## Staged held-ticket issuance — 16 September 2026

The fresh wallet preview now adds a separate native Issue/preview/saved-verification bridge after Book. Book remains unpaid. Existing receipt/list/history surfaces show saved ticket and payment results. See [portal ticket wallet contract](PORTAL_TICKET_WALLET.md) for authorization, gates and recovery limits. Normal Issue adds no PNR call.
