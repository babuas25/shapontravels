# Client API — successful BS journey and BG Issue investigation

**রায়: নতুন BS UAT Book→Issue→report→PNR journey সফল; BG Book সফল হলেও Issue/PNR inconsistent। Unrestricted rollout blocker বহাল। Issue-এর client-facing diagnostics ঘাটতি local code-এ ঠিক করা হয়েছে।**

এই report [BG follow-up](CLIENT_API_BG_FOLLOWUP_2026-09-18.md) ও [remediation report](CLIENT_API_REMEDIATION_2026-09-18.md)-এর পরবর্তী evidence। 18 September 2026-এ পৃথক isolated databases-এ দুইটি independent synthetic traveller দিয়ে **DAC→CGP, 30 October 2026** পরীক্ষা হয়েছে—departure 42 দিন পরে। আগের unresolved booking retry করা হয়নি। প্রতি journey-তে সর্বোচ্চ এক Book ও এক Issue dispatch হয়েছে।

## Live results

| Check | BG | BS |
| --- | --- | --- |
| Search | 200; 4 offers | 200; 8 offers |
| FareRules / Reprice / acceptance / pricing | সব 200 | সব 200 |
| Book / same-key replay | 200 / identical | 200 / identical |
| Issue / same-key replay | 202 / identical | 200 / identical |
| Saved ticket | 202, `outcome_unknown` | 200, verified issued receipt |
| Client ticket report | 409 `VERIFIED_TICKET_REQUIRED` | 200 |
| Client PNR lookup | 502 `SUPPLIER_RECONCILIATION_FAILED` | 200 |
| Supplier Book / Issue dispatches | 1 / 1 | 1 / 1 |
| Additional replay dispatches | 0 | 0 |
| Wallet | BDT 5,557.40 reserved; no final capture | BDT 4,856.20 captured once; hold 0 |

Wallet funds were synthetic and existed only in disposable local UAT databases. BG ledger has one deposit and one `booking_hold`; BS has one deposit, one `booking_hold` and one `booking_confirm`. Hold plus capture is not two charges: BS final available balance decreased by exactly 485,620 minor units from its synthetic deposit and held balance returned to zero. No release or manual settlement was performed for BG.

Local booking IDs:

- BG: `3649fa5b-f230-471a-b28c-21a0db1a4a98`.
- BS: `f5d2c220-f110-4e25-b7e2-93609077ff2f`.

Raw PNRs, ticket numbers, upstream references, passenger details and credentials are excluded from this report.

## BG supplier inconsistency

The saved Book response reports success and returns a PNR. NewTicket then reports `isSuccess=false`, with **“Record locator not found.”** The independently requested PNR lookup also fails. All six actual Issue references exactly match the stored supplier Book response, including `BookingRefNumber=PNR`; no public UUID was accidentally sent as a supplier reference.

Read-only supplier GlobalSearchB2B and AirTicketingDetails both return HTTP 200. Their transaction status is **Ordered**; the detailed report has `statusFor=Ticket`, `isCompleted=true`, a PNR and airline PNR, but no ticket number. Therefore neither Book success nor report `isCompleted=true` proves ticket issuance. The application's unknown Issue and retained wallet hold are appropriate until sufficient evidence resolves it. A supplier rejection does not authorize automatic release or another Issue attempt.

For comparison, the independent BS transaction's supplier report says **Issued** and contains ticket evidence, consistent with the successful client response, report and PNR lookup.

Both new requests explicitly set `isLeadPassenger=true`, use empty `documentType`, a synthetic uppercase/digit document number and a given name without an internal space. Date of birth remains the public API's supported date-only value. This demonstrates that these payloads can Book successfully; it does **not** identify the root cause of the earlier Amadeus validation failure. Flight/date, passenger and supplier timing also changed, so it is not a controlled single-field A/B test. No speculative passenger normalization or requirement was added to application code.

## F9 — Issue 202 diagnostics corrected

Live BG evidence exposed an API inconsistency: Book's unresolved reply supplied client instructions, while Issue's unresolved reply exposed only state/payment and could leave integrators polling indefinitely.

Issue and saved-ticket/replay replies now include:

- `reason`: safe fixed codes, including `SUPPLIER_RECORD_LOCATOR_NOT_FOUND` for the exact observed Triplover response; other supplier failures remain generic.
- `nextAction`: `check_saved_status` only for active pending work; `contact_support` for completed unknown outcomes, pending older than five minutes, and incomplete wallet settlement.
- `automaticRetryAllowed:false` and the owner-scoped ticket `statusUrl`.

Received but unverified evidence uses `TICKETING_RESPONSE_UNVERIFIED`. Missing saved responses use `TICKETING_OUTCOME_UNKNOWN`; this code does not invent a timeout diagnosis because Issue transport errors were not historically retained. A verified ticket whose wallet capture needs recovery returns `WALLET_SETTLEMENT_REQUIRED` while preserving its ticket evidence. Supplier message text and internal details are not exposed.

This is an additive response/contract/documentation change. No migration, supplier dispatch logic, verification threshold, wallet transition or manual-resolution policy changed. The original UAT calls above preceded this correction. Afterwards, existing BG and BS database records were read and replayed through the corrected router **with no real supplier adapter configured**: BG returned the new locator diagnostic/support action, BS retained its settled 200 receipt, and ledger counts were unchanged. This validates saved live evidence without making another booking or ticket request upstream.

## Verification

- Two live UAT journey probes passed their safety assertions. BG's probe passing does not mean its Issue succeeded.
- Two local saved-evidence read/replay probes passed; no supplier transport available, no new ledger entry.
- **40 public response captures / 40 fare-breakdown snapshots** validated against the corrected served contract; zero schema or arithmetic failures. Counts include live captures and local readbacks, not distinct bookings.
- Unit tests cover pending/stale/unknown states, supplier-specific classification, raw-message redaction and status action. Database regressions cover timeout evidence, supplier rejections, passenger/fare/reference verification failures, owner-only status, replay without redispatch and wallet-capture recovery diagnostics.
- Final ordinary tests, full disposable database suite, all-target Clippy, format/diff checks and source hashes are recorded in the [verification manifest](CLIENT_API_ISSUE_FOLLOWUP_VERIFICATION_2026-09-18.json).

Private captures and database archives are retained under `.local/client-api-audit-20260918/bg-explicit-evidence/` and `bs-explicit-evidence/`. Archive listings are checked; restore rehearsal is not claimed. The isolated audit cluster is stopped after verification. Working/production database and deployment remain unchanged.

## Remaining gate

BS now has fresh successful client end-to-end evidence. BG still needs supplier confirmation of the reservation/ticket outcome and an explanation of the contradictory locator/report behavior. A private supplier incident note is prepared but has not been sent. Unrestricted rollout should wait for that issue and target deployment verification; this report does not establish readiness for every carrier, route or live environment.
