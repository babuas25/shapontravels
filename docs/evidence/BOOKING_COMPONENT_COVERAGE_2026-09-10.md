# Saved booking-component coverage review — 2026-09-10

Reviewed all 944 JSON files then present under `.local/evidence/`, recursively examining every `bookingComponents` field, including saved Search/RePrice/booking data and original/selling snapshots. All files parsed successfully. This is an offline file-content audit, not fresh supplier validation or inspection of database dumps/other non-JSON formats.

| Observation | Count |
|---|---:|
| JSON files examined | 944 |
| Files containing bookingComponents | 579 |
| Single-component array occurrences | 133,923 |
| Arrays with multiple components | 0 |
| Empty/null/non-array bookingComponents fields found | 0 |
| JSON parse failures | 0 |

Counts include repeated captures, original/selling copies and embedded snapshots. They are not unique offers or independent supplier observations. Objects with no bookingComponents field do not establish supported pricing coverage. This check measures component cardinality only; it does not revalidate every monetary/tax value or clear historical supplier discrepancies.

No real multiple-component example was found, so allocation implementation is deferred until a supplier sample establishes component/passenger/route coverage. A component must not be assumed to equal a leg. Current single-component pricing remains unchanged. Unlike a null optional brand-content field, unexpected multiple pricing components cannot safely be passed through with guessed markup allocation: the existing `SUPPLIER_PRICING_COVERAGE_UNSUPPORTED` path remains.

Existing regression `unsupported_coverage_and_missing_fares_do_not_mutate_source` passed. It constructs an explicit synthetic two-component offer and verifies `UnverifiedCoverage`; no new test or runtime policy was needed.

Private per-file aggregate audit: `.local/evidence/component-audit-20260910.json`, mode 0600. No raw passenger, credential or reference values copied into this report. No supplier calls, database writes, Book/Cancel/Issue, commit, push or deployment.
