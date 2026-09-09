# Supplier evidence and implementation boundary — baseline, 2026-09-08

Historical baseline. The user subsequently approved production read-only validation. See [production results](PRODUCTION_VALIDATION_2026-09-08.md) for updated credential/fixture evidence and pending decisions.

## Available evidence

The only supplier contract supplied in this workspace is `Triploaver_API_Documentation.md`. The referenced original PDF, Postman collection and full fixture package are absent. `.env` is present; it has not been copied into this report, logged, committed, or used for network validation.

- Lines 82–95 specify separate Triplover UAT Search and API hosts. The environment of configured credentials has not been approved/verified.
- Lines 105–143 describe an already encoded password, `data.token` and the exact typo `tokenExpieryTime`. The adapter preserves these names and passes the configured password unchanged. Applying this shared protocol to Firsttrip/Takeoff is required architecture, not proof that those accounts support every operation.
- Lines 453–495 explicitly label the Search example truncated. It omits `passengerFares`, component coverage and some equivalence attributes. Its dates/reference strings are illustrative, not replayable live evidence.
- Lines 391–451 describe `directions`, per-passenger pricing, class/baggage fields and `bookingComponents`, but do not settle complete carrier/route equivalence, reference TTL, commercial account entitlement or currency semantics.
- Requirements §5.6 supplies authoritative arithmetic examples. They are covered by exact-decimal calculation tests, not represented as actual supplier Search/RePrice fixtures.

## Completed without supplier traffic

The shared internal adapter provides separate hosts, strict expiry parsing, per-connection single-flight token renewal, one read-side 401 refresh, bounded transient read retry/backoff, overall timeout, bounded response body size and redirect rejection. Only read operations are modeled; no supplier mutation implementation exists. Raw responses are internal and are not exposed via public proxy routes.

Mock HTTP tests prove the above transport behaviours locally. They do not prove credential validity, account permissions, supplier idempotency, payload compatibility or fare semantics. Source decimal lexemes are retained with arbitrary-precision JSON parsing.

Protected supplier configuration persists in PostgreSQL with optimistic version checks and audit. The search snapshot and stale-offer validation helpers exist; disabling then re-enabling a supplier does not revive an old unbooked availability epoch. Full Search/Book integration and all-subset selection checks remain pending.

## Evidence needed before final pricing and transaction integration

Provide a complete sanitized fixture package or approve an identified UAT/sandbox account for Login/Search/FareRules/RePrice capture. A useful fixture set includes one-way, return, multicity, connecting/mixed-airline, multiple fare classes, passenger mixes, multiple booking components, nonzero ancillary/service charges, price change and business failures. Preserve numeric precision, unknown fields, null/missing distinctions and relationships between opaque references. Replace credentials, passenger identifiers and document/contact information before storing fixtures in version control.

This evidence is needed for the four business decisions in Requirements §5.7: no-match outcome, rounding, carrier/route/duplicate matching and multiple-component representation. No options have been silently chosen. A decision report should compare the actual observed cases, safe options, their implications and a recommendation before requesting approval. The current truncated sample is insufficient to recommend a business rule.

The established pricing core can resolve audience/scope precedence for a verified airline/route context and compute exact passenger markup, signed discount and aggregate totals. Internal `PendingDecision` signals identify unsupported cases; they are not a final public error/fallback policy.

## Remaining implementation

Full canonical mapping, response projection, markup-management conflict policy, Search/FareRules public integration, RePrice acceptance/versioning, durable booking workflow, idempotency/reconciliation, ticket/report operations and commercial authorization are unfinished. Book/Cancel/NewTicket/direct issue additionally require the commercial execution policy and explicit supplier execution authorization. These steps are not marked complete.
