# TK CNN Book tax audit — 2026-09-08

User explicitly approved the next Book flow after fresh Search/Reprice results and the supplier-side hold uncertainty were disclosed. User requires separate permission for every subsequent flow and prohibits Issue calls/non-refundable bookings.

Exactly one production Triplover Book call was dispatched using the approved fresh TK one-way DAC–IST–SIN 1 November 2026 references and the supplied adult/CNN details. Both saved Search and Reprice had refundable=true and bookable=true; the CNN mismatch persisted. Reprice evidence was less than ten minutes old at dispatch. Direct supplier-adapter diagnostic: no public pricing guard or effective settings were changed and no platform markup was added. Durable create-new intent prevents duplicate dispatch. A local numeric-representation assertion failed before any Book intent or network dispatch; it was corrected to use Decimal comparison, then the single Book was sent.

Book returned isSuccess=true, bookingStatus=Confirmed and a nonempty PNR (stored privately). No ticketInfoes or ticket numbers were present. ticketingTimeLimit is 2026-09-08, date only. This response alone does not establish live hold-only status or rule out supplier-side ticket processing. No PNR, Issue, Cancel or separate email call followed. Supplier contact details were included in Book; automatic supplier notifications are unverified.

| Flow | Component tax BDT | Weighted passenger tax BDT | Missing BDT | Supplier total BDT |
|---|---:|---:|---:|---:|
| Search | 53,979 | 104,958 | 50,979 | 268,772 |
| Reprice | 53,979 | 104,958 | 50,979 | 268,772 |
| Book | 53,979 | 104,958 | 50,979 | 268,772 |

Independent Decimal assertions confirm the CNN omission in all three stages. Book passenger fares equal Reprice passenger fares: adult tax 53,979 and total 147,603.11; CNN tax 50,979 and total 121,168.89. The total correctly includes both passengers and matches 268,772; the component tax field omits CNN. Book flightInfo has no top-level tax/total fields, so its component was compared. This demonstrates persistence in the raw supplier Book response, not an observed undercharge of the total.

Next proposed flow is a read-only PNR query to inspect live booking status and deadline, requiring user permission. No automatic follow-up is configured.

Private evidence: `.local/evidence/tk-cnn-prebook-20260908` (Book request, intent, response, outcome and Decimal audit). Diagnostic: `examples/tk_cnn_book_once.rs`.
