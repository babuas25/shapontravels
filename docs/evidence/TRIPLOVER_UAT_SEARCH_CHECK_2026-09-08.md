# Triplover UAT Search check — 2026-09-08

After credentials were corrected, Login succeeded. Fresh UAT Search for DAC–SIN 1 November 2026, 1 adult and 1 child aged 3, returned 36 offers. Requests used UAT hosts asserted by the diagnostic, with no production references. Carrier filters were empty; the diagnostic selected only TK offers for the intended follow-up.

Carrier counts: BG 1, BS 1, MU 1, UL 4, SQ 1, QR 15, EK 12, MS 1. No TK offers returned in this response; this does not prove TK is unavailable generally or disabled for the account.

Independent Decimal inspection of all 36 offers found no difference between summed component taxes and passenger taxes multiplied by counts. 23 returned offers have both refundable=true and bookable=true, but none has the requested tax mismatch.

The user authorized UAT Book while the search was in progress. Existing constraints remained: do not book non-refundable offers or offers whose tax mismatch has resolved. No eligible TK mismatched offer was available, so no Reprice or Book was dispatched. No Issue, Cancel or PNR call. Active configuration was not modified.

Private evidence: `.local/evidence/uat-tk-cnn-20260908-retry` including Search request/response and Decimal search-audit.json. This UAT result does not supersede production CNN mismatch evidence or verify Book/PNR status mapping.
