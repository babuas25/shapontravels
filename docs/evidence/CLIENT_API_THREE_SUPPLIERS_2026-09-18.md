# Three production suppliers and PNR deadline — 18 September 2026

**Yes: Triplover, Firsttrip and Takeoff each returned live Search data, and all three contributed offers to the combined client response.** The earlier production Hold probe enabled only Triplover. This test explicitly enabled all three Search connections in a fresh isolated database and used the real production credentials from `.env`.

After the user's latest `.env` update, a second fresh run confirmed:

| Route / departure | Triplover retained | Firsttrip retained | Takeoff retained | Final offers | HTTP / partial |
| --- | ---: | ---: | ---: | ---: | --- |
| DAC→CGP / 18 October 2026 | 9 | 14 | 21 | 44 | 200 / false |
| DAC→CXB / 18 October 2026 | 15 | 11 | 24 | 50 | 200 / false |
| DAC→SIN / 21 October 2026 | 160 | 123 | 93 | 376 | 200 / false |

`supplierCount=3` in each final response. Source attribution was checked against the saved offer UUIDs and their `flight_offers.supplier_id`, rather than inferred from airline names. Each adapter also saved its own successful raw supplier response before selection. All nine responses contained successful supplier evidence and offers.

The combined result does not concatenate every original offer. Existing selection compares sufficiently equivalent fares and retains the lowest original supplier total before markup; ties prefer Takeoff, then Firsttrip, then Triplover. Offers without established equivalence remain separate. Therefore fewer retained offers from one source do not imply that its API failed. Live availability changed slightly between runs: the first international response had 377 final offers, the recheck had 376.

This proves the current code and credentials through the local production-mode client router. It does not establish that a separately deployed server has loaded the latest `.env` or enabled all three database Search controls. Credentials alone do not enable a disabled supplier connection. No deployment or working database setting was changed.

## `.env` recheck

The latest file has **zero dotenv parse errors**. The normal application startup path passed `.env` parsing and reached the deliberately unsupported CLI argument's usage error; no server or database connection was started by that check. Blank lines are valid dotenv syntax. The previous invalid lines are now resolved by the user's edit; this task did not rewrite credentials or `.env`.

## PNR deadline priority and Bangladesh local time

The operator explicitly confirmed that the supplier PNR value **`19 Sep 2026, 12:44 PM`** is Bangladesh local time and should take priority over Book's `ticketingTimeLimit`. It resolves to:

```json
{
  "lastTicketTime": "19 Sep 2026, 12:44 PM",
  "lastTicketTimeIso": "2026-09-19T12:44:00+06:00",
  "lastTicketTimeZone": "Asia/Dhaka"
}
```

The review found a real parser defect: `/api/pnr` preserved the named-month value in its raw response, but the saved-deadline and portal-summary parser accepted only `MM/dd/yyyy HH:mm:ss`, so the usable deadline was cleared. The parser now accepts the confirmed named-month format and explicit RFC3339 values, while preserving legacy numeric raw values without inventing their timezone.

The latest verified PNR deadline supersedes the original Book deadline. Missing/invalid latest verified PNR values still clear prior authority; a failed/mismatched lookup cannot erase previous verified evidence. Overlapping lookups retain their existing dispatch-time ordering. Original Book/replay receipts remain unchanged.

An explicit live `/api/pnr` refresh of the existing production Hold returned **200 / Booked**, the exact deadline above and the new normalization fields. The database retained the PNR raw deadline. This refresh made **zero Book, Issue or Cancel calls**. A subsequent explicit PNR refresh can retrieve an updated deadline using the same saved references.

The issue preflight also respects the confirmed PNR instant: a PNR deadline too close or already expired blocks dispatch even if Book's original deadline is later. This behavior was tested only with mocks. No automatic PNR call was added to Issue, and no live Issue was attempted. Book's unknown offset-free formats remain subject to supplier validation rather than inheriting the PNR-specific interpretation.

## Validation and retained evidence

- Two independent three-supplier production Search runs passed; **18 total supplier Search invocations**, all read-only.
- Existing production Hold PNR refresh passed; no new reservation, ticket, cancellation or wallet entry.
- Ordinary tests: **106 passed, 0 failed**. Fresh full database regression passed in **50.94 seconds**, including normalized PNR persistence, earlier PNR deadline priority, expiry refusal before reservation/dispatch, and previous recovery/replay tests.
- All-target Clippy with warnings denied, formatting and diff checks passed.
- **18 public response captures / 956 fare-breakdown snapshots** passed current schema and exact decimal arithmetic validation. This includes both Search runs and retained production Hold/refresh captures; counts are not distinct bookings.
- Source, outcome counts and private archive hashes are recorded in the [verification manifest](CLIENT_API_THREE_SUPPLIERS_VERIFICATION_2026-09-18.json).

Private captures are retained in `.local/client-api-audit-20260918/production-three-suppliers/`, `production-three-suppliers-recheck/` and `production-hold-evidence/`. Fresh archives include the new PNR observation. Archive listings are checked; restore testing is not claimed. The isolated audit cluster is stopped after verification.

Changes are local; there are no new migrations, no deployment and no modification of the application's working database. The earlier real production Hold remains held. Firsttrip/Takeoff Book or Issue capability was not tested by these Search probes.
