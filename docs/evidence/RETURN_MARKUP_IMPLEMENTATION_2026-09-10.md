# Return markup implementation — 2026-09-10

Implemented the user-approved outbound route convention in the shared Search/RePrice matcher. An exact two-route return A→B, B→A uses A→B for one whole-journey markup. A separate B→A one-way keeps its own directional matching and existing audience/scope fallback.

Every returned direction alternative must have the requested endpoints, nonempty segments and continuous connections. All alternatives are preserved. For airline-specific candidates, all segments must agree with the plating carrier and `isCodeShared` must explicitly be false. Airline-independent candidates can use the approved return route even for mixed/codeshare offers; no governing airline is inferred. Unresolved airline-specific mixed/codeshare and open-jaw/multicity cases retain the existing scope guard. The original complex All/All exception remains unchanged.

Current captured fares are whole-journey passenger fares. The existing exact calculation/projection is unchanged, applying a winning rule once per passenger, then multiplying counts. Actual per-leg pricing support remains conditional on verified supplier leg/passenger amounts; no synthetic split or multiplication by segment count was introduced.

## Verification

- Three new unit tests cover outbound versus inbound rule selection, inbound one-way with/without its specific rule, one-time fixed markup, all-alternative checks, mixed/codeshare carrier boundaries, connecting continuity and unresolved open-jaw scope.
- Sanitized return example: original 157,576.05 for 2 ADT + 1 CHD + 1 INF; outbound fixed 500 and inbound fixed 900 configured together. Return selects outbound and produces 159,576.05, preserving directions.
- New disposable-database integration creates/activates both directional rules through Admin APIs and executes mock Search → RePrice → local acceptance. Search and RePrice have identical selling passenger fares and total 159,576.05. Transport permits only Search/RePrice; no supplier Book/Issue is involved.
- All 49 ordinary tests pass (37 unit, 6 foundation, 6 fixture); full PostgreSQL suite passes in 37.28 seconds, including existing booking/reconciliation checks. Strict all-target Clippy, formatting and diff whitespace checks pass.
- Removed disposable `shapon_return_markup_20260910` database after completion. No working/production database migration, new supplier traffic, booking/issue, commit, push or deployment.

Contracts updated in `docs/SEARCH_API.md` and `docs/REPRICE_API.md`. Broader airline/multicity decisions and branded/multiple-component coverage are still separate.
