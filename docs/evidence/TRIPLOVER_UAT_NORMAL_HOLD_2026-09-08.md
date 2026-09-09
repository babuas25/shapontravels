# Triplover UAT normal hold flow — 2026-09-08

User approved an exception to the mismatch-only booking test: a normal refundable UAT offer may be used for Reprice → Book → PNR status verification. No Issue or Cancel authorized/performed.

Fresh UAT Search for DAC–SIN 1 November 2026, 1 adult + 1 child aged 3, returned 36 offers. Ten met the diagnostic's refundable/bookable, passenger-count, single-component and direction constraints. The lowest-priced eligible BG offer was selected. Search and Reprice refundable=true, bookable=true; Reprice isSuccess=true, isPriceChanged=false and unchanged total BDT 59,895.00. Tax BDT 16,568.00 matches weighted passenger tax.

Exactly one UAT Book was dispatched, using supplied private passenger details and fresh UAT references. Durable create-new evidence guards prevent duplicate Book. Book isSuccess=true, bookingStatus=Created, nonempty PNR (private capture), ticketingTimeLimit empty; ticketInfoes absent. The Book component tax and total match Reprice and passenger aggregates. Independent Decimal assertions passed all three stages; Book passenger fares equal Reprice passenger fares.

One PNR query using the six documented reference fields returned isSuccess=false, item1=null, message="Record locator not found." Assertions confirm all submitted reference values match the saved Book response, including BookingRefNumber=PNR as documented. No live status/deadline was returned. Do not infer cancellation, ticketing, or absence of the booking from this failed read; the Book response is preserved. No automatic second Book or cancellation was attempted.

Status findings: this UAT BG Book uses documented Created, unlike earlier production MH/TK Book responses with Confirmed. Because environment, carrier and source differ, this does not prove which factor causes the label variation. The UAT lookup failure could be an upstream lookup/provisioning or timing issue; root cause is not established. Book flightInfo again omits top-level tax/base/total, so the existing public held_response price-shape guard would still reject that shape even though Created passes its status check.

Direct supplier-adapter diagnostic, not a public API end-to-end acceptance run; no markup applied and no core API behavior or active settings changed. UAT hosts asserted before network use. No production references used. Private evidence: `.local/evidence/uat-normal-hold-20260908`; diagnostic: `examples/uat_normal_hold_probe.rs`.
