# Fresh UAT branded-fare availability check — 2026-09-10

User requested confirmation of brand content availability. Performed three direct Triplover UAT Search calls, bypassing platform branded-fare rejection. Exact HTTPS UAT Search/servicing URLs were asserted before access; no database or booking/issue operations were used.

| Search | Dates | Returned offers | Populated brandedFares | Null brandedFares |
|---|---|---:|---:|---:|
| DAC→SIN | 2026-10-25 | 35 | 0 | 35 |
| DAC→DXB→DAC | 2026-10-25 / 2026-11-01 | 13 | 0 | 13 |
| DAC→DXB, DXB→SIN | 2026-10-25 / 2026-11-01 | 16 | 0 | 16 |

One adult, economy, no preferred/prohibited airline filter. Returned plating carriers collectively include BG, BS, EK, MU, QR, SQ and UL. All 64 returned offers have null brandedFares; this is sampled inventory, not all supported inventory.

The supplier's item2 array contains mixed success flags: one-way/return `[true,false,true,true]`, multicity `[false,false,true,true]`. These are partial upstream results, not complete supplier-source success. The failed portions cannot establish brand availability or absence.

The supplied API document lists no separate Brand Content endpoint. It describes optional Search `brandedFares` and FareRules/RePrice `brandedFareRefs`, without a populated brand schema/sample sufficient to implement selection and content mapping. This confirms documented hooks and absence of populated content in the returned sample; it does not prove that branded fares are unsupported or disabled for the account. Account entitlement, eligible inventory, required options, and any separate undocumented endpoint require supplier confirmation. No guessed endpoint probing or support message was sent.

Private original requests/responses and initial aggregate summary: `.local/evidence/uat-brand-20260910T160831464906000/` (directory 0700, files 0600). The partial-status assessment above comes from the raw saved item2 arrays. `examples/uat_brand_probe.rs` is an explicitly gated Search-only probe and now also reports supplier success flags for future runs. No live rerun was needed for the reporting addition.
