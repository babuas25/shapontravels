# Authorized single Book tax audit — 2026-09-08

## Current outcome: ticket status requires supplier verification

Exactly one authorized Book request was sent to the configured Triplover production adapter. Supplier returned `isSuccess=true`, a PNR, and `bookingStatus="Confirmed"`. The subsequent already-started read-only PNR call returned `status="Ticket in progess"` (supplier spelling), with `lastTicketTime="Sep-08-2026 18:00"`.

**This is not certified as a hold-only outcome.** No NewTicket/issue or Cancel call was sent. Book returned no ticketInfoes/ticket-number evidence, but their absence does not prove tickets were not issued or are not being processed. The PNR needs prompt verification with the supplier. PNR is retained privately and shown only to the requesting user, not embedded in this repository report.

Book ticketingTimeLimit is only `2026-09-08`; the PNR deadline has no timezone. The Book intent was recorded at 2026-09-08 12:23:17 UTC (18:23:17 in Dhaka). If the supplier deadline is Dhaka time, it predates dispatch. Do not assume a future hold window or automatic release from these values.

## Scope and exact selection

User supplied five passengers, stated the TEST-prefixed documents were supplier-approved for production use, and explicitly requested one tax-mismatched offer be booked as a hold, without ticket issue. User then supplied contact email/phone, Bangladesh issuing country, and DAC–SIN on 2026-11-01 returning 2026-11-15.

Supplier Search input: 2 adults, 2 children ages 3/11, 1 infant; economy; no carrier filters. DOB mapping: 3-year-old CNN, 11-year-old CHD. Phone was normalized to +880 with a national number without the leading zero. Passenger/contact/document data is private; none is embedded here.

This was a **direct supplier diagnostic**, not the public router flow. The existing public Search guard would reject tax-mismatched offers; it was not weakened. Prices below are supplier amounts, not platform marked-up selling prices. No client/supplier/markup settings or main database schema were changed.

## Flow 1 — Search

902 offers returned. 99 met the diagnostic selection criteria: tax mismatch, bookable=true, one booking component, exactly one direction option per requested route, the expected five passenger counts, and no populated branded fares. The lowest total of those eligible offers was selected, from MH with source marker `#ce25e4`.

| Route | Flight | Supplier local departure | Supplier local arrival |
|---|---|---|---|
| DAC–KUL | MH197 | 2026-11-01 00:50 | 2026-11-01 06:50 |
| KUL–SIN | MH611 | 2026-11-01 09:20 | 2026-11-01 10:30 |
| SIN–KUL | MH608 | 2026-11-15 17:25 | 2026-11-15 18:45 |
| KUL–DAC | MH196 | 2026-11-15 21:40 | 2026-11-15 23:40 |

Search total: BDT 272,530.32. Top-level and component tax: 55,874.00, versus count-weighted passenger tax 108,902.00. Missing tax: **53,028.00**. Reported tax again equals one ADT + one CHD + one INF, omitting count multiplication and CNN.

## Flow 2 — RePrice

One RePrice call on the selected offer succeeded. `bookable=true`, `isPriceChanged=false`. Itinerary, counts, passenger fares and total were unchanged. Both top-level and component tax corrected to **108,902.00**. Single-component pricing coverage was checked before Book, without applying markup or rewriting any supplier tax.

## Flow 3 — Book

Exactly one Book request was sent, using fresh RePrice references and the approved passenger data. A create-new, synced dispatch-intent file was written before the call; the single-attempt mutation adapter does not retry Book. No alternative offer was booked.

Book success and PNR are recorded, but status was `Confirmed` rather than the documented `Created`. The returned flightInfo contains passengerFares, passengerCounts, bookingComponents and directions, **not top-level totalPrice/taxes/basePrice**. Its component taxes and totals reconcile with the passenger values.

| Stage | Top-level tax | Component tax | Weighted passenger tax | Component total |
|---|---:|---:|---:|---:|
| Search | 55,874.00 | 55,874.00 | 108,902.00 | 272,530.32 |
| RePrice | 108,902.00 | 108,902.00 | 108,902.00 | 272,530.32 |
| Book flightInfo | Not returned | 108,902.00 | 108,902.00 | 272,530.32 |

Passenger fares are numerically unchanged across all three stages:

| Type | Count | Tax per passenger | Total per passenger |
|---|---:|---:|---:|
| ADT | 2 | 27,514.00 | 69,464.18 |
| CHD | 1 | 25,514.00 | 63,291.78 |
| CNN | 1 | 25,514.00 | 63,291.78 |
| INF | 1 | 2,846.00 | 7,018.40 |

## Flow 4 — PNR read and communication restriction

One PNR read used the returned booking references. It succeeded but returned `Ticket in progess`; therefore no assertion that the booking is merely held or that ticket issue did not happen is made.

After Book dispatch and after this PNR read was started, the user instructed not to send if it causes email. No further supplier calls were initiated after that instruction. No standalone email/message was sent by the assistant. The already-dispatched Book included the user-provided contact email; automatic supplier email delivery is unknown and cannot be ruled out or retroactively prevented. No cancel or issue operation was attempted.

## Evidence and limits

Private `.local/evidence/hold-tax-20261101`: request/response captures, selected fare, private passenger file, durable Book intent/outcome, PNR read, independent Decimal audit and SHA-256 manifest. Captures are 0600 inside a 0700 directory. `examples/authorized_hold_probe.rs` contains no passenger PII and is explicitly gated; its Book phase cannot overwrite existing request/intent files. Do not rerun Book for this intent.

Independent Decimal assertions confirm the Search mismatch and matching RePrice/Book component taxes, total, base, AIT and passenger fares. This establishes **Search-only tax mismatch for this selected five-passenger offer**; it does not resolve prior CNN-only cases, certify hold-only status, or validate the public platform's Book success classifier. Supplier confirmation of status and deadline is outstanding. No external message was sent to request that confirmation.
