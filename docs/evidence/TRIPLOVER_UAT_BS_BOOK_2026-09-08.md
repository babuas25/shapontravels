# Triplover UAT BS booking — 2026-09-08

User explicitly requested a BS airline UAT booking. Fresh Search → Reprice → exactly one Book completed for DAC–SIN 1 November 2026, 1 adult and 1 CNN child aged 3, using the previously supplied passenger details. Search returned 36 offers and one eligible BS offer under the diagnostic constraints. Both Search and Reprice refundable=true, bookable=true; Reprice isPriceChanged=false.

Book isSuccess=true, bookingStatus=Created, and returned a nonempty PNR (private evidence). No ticketInfoes in Book. No Issue, Cancel or PNR query was made. Book result alone is not an independent live-status lookup. No duplicate Book or automatic retry.

Search, Reprice and Book component tax all BDT 18,558.00; weighted passenger tax agrees. Supplier total BDT 61,886.00 in all three stages and matches weighted passenger totals. Book passenger fares equal Reprice passenger fares. Independent Decimal assertions passed.

UAT host assertions enforced. Direct supplier-adapter diagnostic, no production references, no public API/markup projection claim, no active settings changed. Evidence: `.local/evidence/uat-bs-hold-20260908`; diagnostic: `examples/uat_bs_hold_probe.rs`.
