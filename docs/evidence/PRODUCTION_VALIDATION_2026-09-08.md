# Production read-only validation — 2026-09-08

**Subsequent decision:** User approved passenger-level two-decimal half-up rounding and no-rule pricing configuration error. These are implemented and tested. The user also subsequently approved rejecting same-scope active duplicates with 409; versioned admin rule management now enforces that policy. The recommendations/pending descriptions below record the evidence at capture time; see REQUIREMENTS.md §5.7 and its approved implementation update for current policy.

The user explicitly authorized the updated production `.env` for the next read-only integration step and prohibited ticket issue. Only Login, Search, FareRules and RePrice were called. No Book, Cancel, NewTicket, direct issue or PNR was sent. Existing admin/client records and supplier activation settings were not changed. The validation example uses the configured connections independently of the public Search activation settings; it is a manual diagnostic, not a commercial Search endpoint.

## Results

| Connection / search | Passengers | Offers | FareRules | RePrice |
|---|---|---:|---|---|
| Firsttrip: DAC → CXB, 2026-09-29 | 1 adult | 27 | Success | Success, BDT |
| Takeoff: DAC → CXB, 2026-09-29 | 1 adult | 24 | Success | Success, BDT |
| Triplover: DAC → CXB, 2026-09-29 | 1 adult | 27 | Success | Success, BDT |
| Triplover: DAC → BKK → SIN, 2026-09-29 / 2026-10-06 | 2 adults, child age 6, infant | 467 | Success | Success, BDT |
| Triplover: DAC ↔ SIN, 2026-09-29 / 2026-10-06, SQ filter | 2 adults, child age 6, infant | 7 | Success | Success, BDT |

Each follow-up used the first returned offer's real transaction/item references and all segment references in order. Only one selected offer per successful search was followed through FareRules/RePrice. This is sampled validation, not a guarantee that all airlines, routes, fares, accounts or operations work.

An initial unrestricted return Search exceeded the adapter's 8 MiB response bound. A narrower SQ request succeeded. The cap remains bounded and unchanged; unrestricted large-result support remains an operational limitation requiring payload-size/performance work. No repeated mutation or booking fallback was used.

Private full response captures are under the Git-ignored `.local/evidence/` directories `20260908T070222Z`, `20260908T070334Z`, and `20260908T070447Z`. Files are mode 0600 in mode 0700 run directories. No login tokens/passwords were written by the probe. Failed size-limit response contents were not saved.

## Observed contract differences and pricing evidence

1. Search `item2` is an ARRAY of upstream status objects, including mixed `isSuccess` values. It is not the object shown in the abbreviated supplier document. Valid offers coexist with unsuccessful upstream entries; the probe reports array shape and offer count rather than interpreting a nonexistent `/item2/isSuccess`. FareRules/RePrice use the object status form in the checked samples.
2. Search offers have no `currency` field. The five selected RePrice responses report BDT. This does not establish currency for every Search offer or prove that account currency can never change. A trusted account currency contract is still required before comparing totals across connections; no field was invented in Search responses.
3. All captured offers have ONE booking component, including the two-route return/multicity offers. There are 138 mixed-airline offers in the 467-offer multicity result. Components cannot be inferred from route count, and markup carrier matching cannot be inferred from the first segment.
4. In the three domestic searches and multicity result, every top-level total equals the sum of per-passenger totals times counts. Passenger total prices have two decimal places in those captures. Representative fixed-500 projection tests reconcile passenger/component/offer totals and preserve all other values and JSON shape.
5. Example Triplover adult fare: supplier total 4033.05, base 3224.00, taxes 1125.0, AIT 0.0, original supplier discount -315.95. The established platform formula with Fixed 500 gives selling total 4533.05 and discount -184.05. The supplier's original signed discount is not used as an incremental markup basis.
6. A hypothetical 3% markup on the observed 4033.05 produces 4154.0415, which exceeds observed passenger precision. The fixture test deliberately returns an internal rounding-decision signal; no unapproved public rounding behaviour is exposed.

## Decisions required by Requirements §5.7

| Decision | Safe options and effects | Recommendation, awaiting approval |
|---|---|---|
| Rounding | Keep all calculated decimal digits, or round individual passenger selling amounts to two decimals and then aggregate counts. Rounding only the final aggregate can disagree with displayed per-passenger totals. | For the observed BDT flows, two-decimal half-up at passenger level, then derive discount and aggregate from that rounded value. This matches observed precision and makes totals reconcile. Other currencies require their own precision contract. |
| No matching rule | Reject pricing until an admin supplies a rule, or explicitly allow zero markup. The second option exposes supplier pricing with no margin. | Return a pricing-configuration error and require an applicable rule. No silent supplier-price fallback. |
| Carrier/route matching and duplicates | Match whole requested journey with a defined carrier policy, or limit initial supported specific scopes and use approved All/All rules for complex journeys. Duplicate config can be rejected or explicitly version-replaced. | Mixed-airline evidence rules out an implicit first-leg shortcut. A concrete matching/management policy still needs a separate decision; no default has been selected. |
| Multiple components | Derive coverage from actual supplier components or defer those offers. Equal/proportional splitting without evidence may corrupt prices. | Keep multi-component projection deferred until such a response is captured and its coverage established. No multi-component live sample occurred in this run. |

Two immediate user questions (rounding and no-match policy) have been raised. Until approval, `projection::single_component` is an internal tested helper, not a public commercial response pipeline. No-match/duplicate signal types remain internal; they are not final business error codes.

## Code, fixtures and verification

- `examples/supplier_probe.rs`: opt-in `--production-read-only` manual diagnostic. Current return/multicity presets narrow carriers to SQ/TG to bound future traffic; the captured multicity run preceded that filter and was unfiltered. Changing presets does not rewrite captured requests.
- `examples/export_fixture.rs`: preserves JSON numbers with arbitrary-precision parsing; replaces non-allowlisted nonempty strings with deterministic SHA-256 pseudonyms. Search fixture arrays deliberately retain only the first three offers. Full raw captures stay private. These are representative subsets, not full Search response replicas.
- `tests/fixtures/production/`: sanitized Search, return, multicity, FareRules and RePrice evidence. FareRules narrative strings are pseudonymized, so these fixtures test structure rather than penalty text semantics.
- `src/projection.rs`: immutable single-component projection with exact arithmetic, coverage/ancillary validation and deferred extra precision. It does not manage pricing versions or issue a ticket; lifecycle integration must enforce that callers supply the original private supplier snapshot and do not apply markup twice.
- `tests/production_fixtures.rs`: recursive shape and nonpricing value preservation; Fixed 500 once per individual passenger across return/multicity; currency/reference preservation; missing fare, invalid service charge, ancillary, component coverage and rounding guards.

Public Search, canonical equivalence/lowest selection, markup rule management, public RePrice and transaction workflows remain unfinished. No new public flight endpoint is advertised as complete.
