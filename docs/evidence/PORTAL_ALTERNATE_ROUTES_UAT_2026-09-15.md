# Alternate routes — actual Triplover UAT, 15 September 2026

**Result:** A fresh Bangkok return search completed through the actual frontend → Rust → Triplover UAT as a real Hold for Nadia Rahman, PNR **0A4OYD**, payable **BDT 41,932.00**. The supplier PNR recheck returned `Booked`. No ticket was issued. The earlier family-fare timeouts and additional findings below remain unresolved.

## Environment

The existing signed-in `shopontravels` frontend at `localhost:3000` and Rust `shapontravels` backend at `127.0.0.1:18081` were used with the existing updated Triplover UAT credentials. Additional read-only searches went directly through the same authenticated Rust bridge. No production booking, ticket issue or payment was performed.

## Search results

| Case | Travel date(s) | Passengers | Actual result |
| --- | --- | --- | --- |
| DAC → CGP | 30 Sep 2026 | 1 adult | Frontend displayed 13 flights: 9 US-Bangla and 4 Biman. Rust stored 13 offers. |
| DAC → BKK | 30 Sep 2026 | 1 adult | Rust HTTP 200, 9 holdable offers from BS/BG/SQ; 117.3 seconds. |
| DAC → KUL | 30 Sep 2026 | 1 adult | Rust HTTP 200, 6 holdable offers from BS/SQ; 117.8 seconds. |
| DAC → BKK | 30 Sep 2026 | 1 adult, child age 6, 1 infant | Actual frontend displayed 15 schedule options from 9 Rust offers. Rust stored the correct passenger counts. |
| DAC → BKK → DAC | 30 Sep / 6 Oct 2026 | 1 adult | Rust HTTP 200, 13 holdable return offers from BS/BG/SQ; 36.0 seconds. |
| DAC → CGP, CGP → DAC, DAC → CXB | 30 Sep / 1 Oct / 2 Oct 2026 | 1 adult, child age 3 | Rust HTTP 200, 9 holdable three-leg offers from Biman; 35.8 seconds. |

An offer can contain several schedule options, so offer counts and displayed flight counts differ. The earlier SIN/CCU failures do not mean all international routes fail. The BKK/KUL searches succeeded close to the configured 120-second supplier deadline; the later return/multicity calls were faster. These measurements do not establish the cause of the earlier timeouts.

## International Hold preparation

The actual frontend selected THE CITY FLYERS as owner, then tried the DAC → BKK family fares for 30 September:

- US-Bangla BS-217, 10:00–13:40, booking class K, displayed gross BDT 56,501: price check timed out and showed “The result could not be confirmed. Reload the current state before retrying.” The draft has no saved price or booking.
- Biman BG-388, 11:05–14:50, booking class K, displayed gross BDT 65,800: price check also timed out with the same frontend message. Its draft likewise has no saved price or booking.

Direct authenticated Rust RePrice controls succeeded for the US-Bangla Bangkok return fare (HTTP 200, holdable, 41.5 seconds) and the Biman three-leg domestic fare with a three-year-old child (HTTP 200, holdable, 44.5 seconds). This does not resolve the two family-fare timeouts or prove the frontend Hold preparation path for those cases. The raw RePrice evidence is retained alongside the search responses.

No price was accepted and no new supplier Book was submitted during these initial search/price checks. A successful Search with `bookable=true` does not itself prove RePrice or Hold completion. These attempts use the signed-in Super Admin on-behalf flow; a real signed-in B2B self-service Hold is still unverified.

## Confirmed display gap

The Bangkok return response contains 55 route combinations across 13 offers. One offer contains 20 combinations. The current `shopontravels/lib/rust-flights/presentation.ts` cap rejects that entire offer because it exceeds 12. This provides a real UAT reproduction of the previously code-only limitation; 20 combinations would be omitted by that cap before overlap/ambiguity filtering. The domestic multicity response has 40 combinations, with at most 8 per offer, so it does not hit the cap.

The fresh return search in the browser reproduced this warning and displayed 35 options. The selected direct US-Bangla fare was available and was not part of the omitted offer.

## Completed international return Hold

| Field | Verified value |
| --- | --- |
| Owner / creator | THE CITY FLYERS / signed-in Super Admin |
| Test passenger | Ms Nadia Rahman, one adult |
| Outbound | BS-217, DAC → BKK, 30 Sep 2026, 10:00–13:40 |
| Return | BS-218, BKK → DAC, 6 Oct 2026, 14:40–16:20 |
| Supplier / gross / commission / payable | BDT 41,707 / 42,207 / 275 / 41,932 |
| Tier | Basic, 55% commission share of the BDT 500 markup |
| PNR / airline PNR | 0A4OYD / 0A4OYD |
| Public reference | STR0A4OYD0A4OYD |
| Draft / booking UUID | `70314815-49ea-406c-85d7-1ce704c33567` / `171b82e7-f4c0-4a9f-a696-dedebc654f7d` |
| Stored state / execution | `held` / `hold` |
| PNR check | HTTP 200 in 3.2 seconds, `Booked`, success true, manual resolution not required |
| Latest supplier time limit | `09/15/2026 19:55:00`; timezone unspecified. The initial Book response said `15/09/2026 20:40:00`; the verified PNR result supersedes it. |

The frontend explicitly accepted the owner-specific price before opening the form. Name, DOB and passport fields began blank. Passport fields were visible and required for the international itinerary. Continuing with missing details showed validation errors for name, DOB, passport number, passport expiry and phone. The DOB picker stopped at the adult cutoff; the passport picker disabled dates before 30 December 2026. Entering the normal test passenger details completed the badge and allowed Review. The save-passenger checkbox remained unchecked.

One **Place booking On Hold** click created the booking. The permanent receipt survived reload, displayed both itinerary legs, DOB/passport/expiry and accepted payable, and appeared in the original Booking Management Ticket table. The database confirmed the immutable creator/owner and the entered customer contact. A separate read-only PNR lookup confirmed the supplier reservation. Receipt refresh itself still reads saved state only.

The local UAT database now contains two Book records including the earlier domestic Hold, with zero ticket issue attempts and zero ticket verifications. No retry of the new Book was sent. The two family-fare drafts remain unpriced and unbooked.

## Additional issues noted

- **Cabin mismatch from supplier inventory:** The request used `cabinClass=1` (Economy), but the selected US-Bangla offer's Search and RePrice segment fields said `Business`, booking class I. The receipt preserves that supplied value; the Book response's corresponding cabin field is absent. The actual cabin cannot be inferred from the requested search class alone. The results should make a returned cabin differing from the requested class explicit before acceptance, or apply an agreed cabin-matching rule. No data was relabeled to make this test appear consistent.
- **Return route formatting:** Booking Management displays `DAC → BKK → BKK → DAC`, repeating the shared airport between legs. The receipt and itinerary retain the correct two flights. This is a list formatting issue.
- **Family-fare timeouts:** Both attempted one-adult/child/infant fares timed out during Hold preparation. That mixed-passenger Hold remains unverified. Successful return and multicity RePrice controls do not establish the cause of those timeouts.
- **Remaining UAT:** A real signed-in B2B self-service Hold and completed child/infant/multicity Holds remain outstanding. This new success covers an international return Hold through Super Admin on-behalf booking.

## Evidence

Private raw requests, responses and timing summaries are retained in ignored `.local/evidence/portal-uat-other-routes-20260915/`. No credentials or tokens are stored in those evidence files. The frontend results and error states were read through the actual browser. Database checks distinguish unsubmitted drafts from supplier Book records.
