# Complex markup scope review — 2026-09-10

Status: evidence review complete; proposed business policy awaits user decision. No runtime matching policy changed.

Later user clarification: return outbound-route matching is now approved; per-leg calculation is requested conditionally on actual leg/passenger pricing. Mixed-airline and multicity proposals are still unapproved. See [latest fare review](PASSENGER_LEG_FARE_REVIEW_2026-09-10.md); it supersedes the corresponding pending/blanket calculation statements below.

## Current implementation

`src/search.rs::matching_context` is shared by Search and RePrice. Specific rules currently require one requested route, one direction option, one segment, matching segment/plating carrier and matching endpoints, without a true `isCodeShared` flag. Otherwise only exclusively All/All candidate rules are permitted. Any scoped candidate returns `COMPLEX_SCOPE_MATCHING_UNRESOLVED`, including a candidate that might ultimately be irrelevant under a future policy.

The established resolver already implements Agent-before-B2B fallback, the four scope priorities, one winning rule, exact per-passenger markup and half-up rounding. These rules need no replacement. The missing decision is how a whole supplier fare maps to an airline and route for that resolver.

## Offline evidence

Inspected 18 saved supplier Search responses in private directory `.local/evidence/reprice-contract-20260910-5pax/`, matching `*search*raw.json`. Each file wraps its supplier payload under `response`. This is the main matrix only; it excludes the separate MH focus run. No new supplier calls were made.

Across 7,104 offer occurrences (repeated inventory across calls, not unique flights):

| Observation | Occurrences |
|---|---:|
| At least one direction option has multiple segments | 7,008 |
| More than one distinct segment `airlineCode` across the offered options | 2,484 |
| At least one route has multiple direction alternatives | 1,288 |
| Supplier `isCodeShared` is true | 7,008 |

Counts overlap. The mixed count is across all offered options, not proof that every selectable combination is mixed. A true codeshare flag does not by itself establish which airline should govern our markup. These observations describe captured payloads, not current inventory or verified supplier commercial policy.

Concrete examples, with no passenger data or supplier references:

- `oneway-all-triplover-search-0-raw.json`, zero-based offer 44: requested DAC→SIN; segments DAC→KUL with MH and KUL→SIN with SQ; `platingCarrier=MH`. Matching MH, SQ, or both would have different commercial effects.
- `roundtrip-all-triplover-search-6-raw.json`, offer 131: DAC→KUL→SIN and SIN→KUL→DAC, with MH/SQ segments and plating MH. A DAC→SIN rule and SIN→DAC rule could both exist with different amounts.
- `multicity-all-firsttrip-search-12-raw.json`, offer 40: requested routes DAC→SIN, KUL→DAC, DAC→MLE. The first route includes MH/SQ segments; plating MH. Treating the first origin and final destination as the whole route would incorrectly invent DAC→MLE as the sole commercial context.
- Sanitized `tests/fixtures/production/triplover-return.json`: SQ DAC→SIN and SIN→DAC. Even with one airline, the route rule remains a business decision.
- Sanitized `tests/fixtures/production/triplover-multicity.json`: TG DAC→BKK and BKK→SIN. Whole-fare pricing does not establish whether the first or second route should control markup.

## Options and effects

1. **First requested route + supplier plating carrier (recommended proposal).** One-way connections use the requested origin/destination, excluding transit airports. Return and multicity use the first requested route. All journey types use the returned plating carrier. This gives one deterministic scope with the current Admin rule schema. It deliberately means later multicity routes do not select a separate markup, and segment carriers do not select airline rules.
2. **Any requested route / segment airline.** Recognizes more matching rules but can produce multiple winners at the same existing priority. Requires an additional explicit conflict policy; choosing the largest/smallest markup, first segment or latest edit is not currently authorized.
3. **Explicit whole-itinerary scopes.** Configure ordered route lists and potentially carrier combinations. More expressive, but adds Admin schema, management and fallback decisions beyond the existing single airline/origin/destination fields.

The recommendation is a platform business convention, not a supplier-confirmed standard. Supplier payloads establish the ambiguity; they cannot choose the user's pricing policy.

## Concrete proposed contract — not active

- Use the supplier offer's nonempty valid `platingCarrier` for airline scope on all journey types, including mixed-airline and codeshare offers. Do not infer it from a segment when missing or contradictory.
- Use the first requested route's ordered origin/destination for route scope. For a connection DAC→KUL→SIN requested as DAC→SIN, the scope is DAC→SIN. For a return DAC→SIN→DAC, the scope is DAC→SIN. For multicity DAC→SIN, KUL→DAC, DAC→MLE, the scope remains DAC→SIN.
- Routes remain directional: a separate one-way SIN→DAC does not match DAC→SIN. Do not automatically reverse-match.
- Apply the existing audience/scope priority once to the whole passenger fare. No per-leg addition, markup stacking, or change to lowest original-supplier-price selection. Fixed 500 with five passengers adds 2,500 for the whole journey, not 2,500 per leg.
- Validate all offered alternatives against their requested route endpoints and segment continuity; do not select only the first alternative or discard mismatches silently. Preserve the full response and original reference selection.
- Search and RePrice use the same policy. RePrice resolves current rules and stores the resulting version; Book retains the accepted snapshot without recalculating markup. Existing branded/multiple-component coverage limits remain separate.

Implementation after approval: extend matching context with route/alternative validation, exercise Search and RePrice against direct/connecting/return/multicity/mixed fixtures, verify audience fallback and unchanged one-time pricing, and update the public documentation. No migration is expected for option 1. Supplier mutation and release are separate work.

## Verification and boundary

- Existing `complex_all_scope_keeps_agent_priority_and_blocks_specific_rules` test passed, confirming the current All/All exception and scoped-rule guard. This is not evidence that the proposed extension is implemented.
- Offline field-only analysis produced the counts/examples above. No raw captures, passenger data or opaque references copied into this document.
- No supplier traffic, database mutation, booking, issue, deployment or runtime policy change.
- REQUIREMENTS.md §5.7 explicitly requires an evidence report and user approval before locking unresolved route/carrier matching. The user's instruction to proceed authorizes this review; it does not choose among these previously unresolved commercial policies.
