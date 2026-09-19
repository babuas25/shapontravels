# Agency API → admin end-to-end test — 18 September 2026

Result: the tested request and quotation flows passed. No ticket was issued, reissued, refunded or voided. No supplier adapter or notification delivery worker ran.

## Scope and isolation

The main local database contained six bookings but no eligible issued, wallet-backed ticket. Testing therefore used a separate database cloned from existing synthetic workflow fixtures, a Rust HTTP server on `127.0.0.1:18082`, and a copy of the actual frontend components and API routes on `localhost:3001`.

The test used real machine-token exchange, authorization, HTTP requests, database transactions and browser-driven admin controls. The admin identity provider/session subject was synthetic; this does **not** validate a real Clerk sign-in, production deployment, live supplier execution or email/SMS delivery.

The isolated Rust harness (`examples/ticket_management_e2e.rs`) has no supplier adapters, starts no delivery workers, restricts database names to the synthetic E2E prefix/suffix, binds to loopback, and denies ticket execution and settlement-completion routes. Existing synthetic ticket evidence was reused without changes.

## Observed results

| Scenario | Result |
| --- | --- |
| Machine token exchange | HTTP 200 |
| Ticket request list before permission grant | HTTP 403 |
| Admin API Management grants ticket-management read/write | Saved through the actual frontend; Booking, Ticketing and Cancellation permissions remained unchecked |
| Agency availability and reissue submission | HTTP 200/201; passenger, route and requested departure preference preserved |
| Exact request replay | HTTP 200, replay true; no second request |
| Admin request queue and detail | Agency name/code and request visible |
| Admin accepts and publishes reissue quote | BDT 10 fare difference + BDT 5 airline fee + BDT 2 service fee |
| Agency approves quote, then repeats decision | HTTP 200 for both; exactly BDT 17 reserved once; no terminal ticket outcome |
| Agency refund submission → admin quote → agency rejection | BDT 50.02 entitlement less BDT 7 fees = BDT 43.02 quoted credit; customer-rejected outcome; no credit or refund execution |
| Agency void submission → admin rejection | Visible in Requested queue, then rejected without voiding a ticket |
| Release unperformed reissue hold | BDT 17 restored; test wallet returned to its original available balance and zero hold |
| Main local booking/ticket/wallet records | Fingerprints unchanged |
| Isolated bookings, issue/verification/cancellation evidence and entitlements | Fingerprints unchanged throughout; zero new issue/cancellation records |

Synthetic references: reissue `TMRE00000000000E`, refund `TMRR00000000000F`, void `TMRV000000000010`.

## Defect found and fixed

After an agency API decision, Refresh updated the admin queue but left an open detail panel showing the previous quotation state. `shopontravels/components/dashboard/ticket-management/TicketManagementWorkspace.tsx` now reloads detail when the selected request version changes, ignores superseded list/detail responses, and renders detail only for the selected request.

Browser regression verification kept the refund detail open while the agency rejected its quotation through the API. Refresh changed that same panel to Rejected with Customer Rejected activity and removed the waiting/Reject controls. Rejecting the void request in the Requested queue removed both its row and detail panel. Reissue approval and hold release were also verified in the browser.

## Validation and local activation

- Frontend `tsc --noEmit`: passed.
- Targeted ESLint for `TicketManagementWorkspace.tsx`: passed.
- Rust `cargo clippy --locked --example ticket_management_e2e -- -D warnings`: passed.
- Local frontend release activated at rollout revision 13, schema 52, with backup/restore verification; no schema migration was required.
- Notification settings and existing agency API permissions in the main app were preserved. This test does not enable permissions for real agencies.

Private HTTP evidence, before/after fingerprints and test helper scripts are retained under `.local/ticket-management-e2e/`; release evidence is under `.local/ticket-management-e2e-release/`. The isolated browser tab and services were closed after testing. The normal local application remains on ports 3000/18081.
