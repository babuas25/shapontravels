# First-segment airline markup — 2026-09-10

The user explicitly corrected the proposed plating-carrier convention. The governing airline is now the first flight segment's `airlineCode` in the first requested route. For DAC→SIN via KUL with MH then SQ, use MH + DAC→SIN, regardless of plating carrier. Markup still applies once to the whole-journey passenger fare, with unchanged priority, fallback, rounding and count aggregation.

Search and RePrice share the same matcher. It validates all requested route endpoints, alternatives and continuous segments. First-airline codes must be valid two-character uppercase alphanumeric identifiers to select airline rules. Different first airlines across first-route alternatives remain ambiguous for airline-specific pricing; none is arbitrarily chosen. Airline-independent candidates can still use the validated first route. Supplier selection, original references, plating metadata and fare shapes are not rewritten.

Unit coverage checks a synthetic connecting MH→SQ itinerary with SQ plating: MH wins over a competing SQ rule. Null/empty/malformed first-airline values fail airline-specific matching. Conflicting alternatives remain guarded; route-only rules and malformed-route rejection are preserved.

Disposable PostgreSQL coverage extends both captured return and multicity examples with synthetic mixed-airline variants. First segment MH competes with an actual plating-carrier rule at the same route scope (fixed 500 versus 1,200), plus a later-route rule. Admin rule creation/activation → Search → RePrice → local acceptance must retain the first-airline fixed-500 total. These are explicit mock mutations of sanitized fixtures, not newly observed supplier responses.

The interrupted plating-based draft was corrected before verification/release. No supplier traffic, booking/issue, new migration, working/production database changes, commit, push or deployment.

Final verification: all 51 ordinary tests pass (39 unit, 6 foundation, 6 fixture), full disposable PostgreSQL suite passes in 37.52 seconds, and strict all-target Clippy, formatting and diff whitespace checks pass. Disposable `shapon_first_airline_20260910` database removed.
