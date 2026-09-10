# Hold continuation — 2026-09-10

User requested the sequence Hold → booking/PNR retrieval → Ticket Issue → Direct Issue. Production mutation remains prohibited. No new Hold, Cancel or Issue was dispatched in this increment.

A fresh Triplover UAT PNR read used the original six saved references from the successful 2026-09-08 hold. It again returned success=false, `Record locator not found`, with no PNR status or deadline. Private evidence: `.local/evidence/uat-pnr-recheck-20260910T172008199582000/`. The read-only example now creates a new private directory per run without overwriting prior captures. No reference substitution, mutation retry or manual resolution was performed.

The separate 2026-09-10 Book attempt is still outcome_unknown following a duplicate-booking business failure with no PNR. Neither observation proves no booking exists or authorizes treating it as cancelled.

## Public Hold compatibility fix

The earlier successful UAT Book had flightInfo passenger fares and a component, but omitted top-level total/base/tax. The public validator incorrectly required those aggregate fields. It now allows their omission while retaining all passenger/count/component/itinerary comparisons to the accepted RePrice. Present nonnumeric/null/mismatched aggregates still fail. Missing aggregate fields remain absent in the public response; existing passenger/component totals receive stored selling amounts without applying markup again.

Integration cases cover omitted aggregates yielding a held result, mismatched component total yielding outcome_unknown, explicit null aggregate yielding outcome_unknown, and identical same-key replay without another mock Book dispatch. This is local compatibility coverage, not a live successful Hold/PNR claim.

No working/production database update, migration, commit, push or deployment. Ticket issue and direct issue have not been implemented or executed by this increment; successful retrieval, current deadline and commercial authorization remain necessary for the subsequent issue stage.

Validation: all 51 ordinary tests, strict all-target Clippy, formatting and full disposable PostgreSQL integration pass (41.30 seconds). Removed `shapon_hold_shape_20260910` after verification. The only real supplier call in this increment was the UAT PNR read.
