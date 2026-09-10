# Multicity first-route markup — 2026-09-10

The user approved matching only the first requested route after confirming captured multicity responses price the whole journey. The shared matcher now handles two or more requested routes, retaining all-alternative endpoint/continuity checks and the existing conservative airline-specific guard.

For DAC→SIN, SIN→BKK, BKK→DAC, the route scope is DAC→SIN. A later BKK→DAC rule neither changes the first route's percentage nor gets averaged into it. If the first route lacks a matching rule, existing audience/airline/All Routes fallback applies; if none applies, pricing fails with `PRICING_CONFIGURATION_ERROR`. This does not select the first later route with a configured rule or an intermediate connecting segment.

Markup still applies once to each passenger's whole-journey fare, with exact half-up rounding and count aggregation. No fare split or per-leg calculation was introduced. Mixed/codeshare airline-specific rules remain unresolved unless the existing carrier checks establish an unambiguous context; route-only rules do not need a governing-airline decision.

Verification includes a synthetic three-route case with 5% on the first route and 3% on the last: the first wins, and the adult whole fare 54,121.33 becomes 56,827.40. Removing the first rule does not select the later rule; adding an All/All rule verifies normal fallback. Existing return and malformed-alternative tests remain in place.

Disposable PostgreSQL integration now exercises both captured return and multicity fixtures through Admin rule creation/activation → Search → RePrice → local acceptance, with different first/later route rules configured simultaneously. The captured multicity original 162,437.94 becomes 164,437.94 for four passengers at fixed 500; refreshed selling passenger fares and totals match Search.

No supplier calls, booking/issue, new migration, working/production database changes, commit, push or deployment. Contracts updated in Search/RePrice documentation; earlier multicity-scope-pending notes are superseded only within this coverage.

Final checks: all 50 ordinary tests pass (38 unit, 6 foundation, 6 fixture); full disposable PostgreSQL suite passes in 37.46 seconds. Strict all-target Clippy, formatting and diff whitespace checks pass. Disposable `shapon_multicity_markup_20260910` database removed after verification.
