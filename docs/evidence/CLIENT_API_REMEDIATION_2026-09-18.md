# Client API remediation — 18 September 2026

**রায়: code-level security/contract fixes যাচাই হয়েছে, কিন্তু supplier booking reliability না মেটা পর্যন্ত unrestricted client rollout আটকে রাখা উচিত।**

পরবর্তী independent BG run-এ timeout-এর বদলে 3.357 seconds-এ Amadeus invalid-data failure পাওয়া গেছে। Search→acceptance সফল, Book `202`, replay-safe, Issue/debit হয়নি। [BG follow-up report ও evidence](CLIENT_API_BG_FOLLOWUP_2026-09-18.md) দেখুন; নিচের two-run counts আগের verification snapshot-এর।

সর্বশেষ [Issue follow-up](CLIENT_API_ISSUE_FOLLOWUP_2026-09-18.md)-এ নতুন BS Book→Issue→report→PNR সফল ও একবার wallet capture হয়েছে। BG Book সফল হলেও Issue/PNR inconsistent; Issue diagnostics-ও local code-এ যোগ হয়েছে। নিচের “fresh successful Issue হয়নি” কথাটি আগের snapshot-এর, সর্বশেষ অবস্থার জন্য নতুন report দেখুন।

এই পরিবর্তনগুলি [আগের audit](CLIENT_API_AUDIT_2026-09-18.md)-এর follow-up। Source baseline `2d2c21f7ed346c906ddbe6f42267f83fa8335225`; fixes বর্তমান local working tree-তে। Working/production database বা deployment পরিবর্তন করা হয়নি। নতুন migration নেই।

## পরিবর্তন ও regression evidence

| Finding | পরিবর্তন | যাচাই |
| --- | --- | --- |
| F1: queued Book-এর stale permission | Reservation lock পাওয়ার পরে current active state, booking/ticketing grant ও managed API eligibility পুনরায় পরীক্ষা | Revoke commit-এর পরে queued Book `403 CLIENT_BOOKING_DISABLED`; supplier Book calls **0** |
| F2: Reprice দিয়ে pool আটকে রাখা | Transaction-এর আগে one-per-client admission; process limit `min(4, floor(pool/2))`; offer lock `NOWAIT` | দুই-connection pool-এ slow Reprice চললেও concurrent Reprice `503 REPRICE_BUSY`, `/auth/me` **200**; external offer lock-ও এক সেকেন্ডের মধ্যে busy; permit পরে পুনর্ব্যবহারযোগ্য |
| F3: machine token requests দিয়ে Admin login বন্ধ | Machine/Admin-এর পৃথক source/identity quotas ও password-verification capacity; malformed secret quota-এর আগে reject | 120 oversized secret-এর পর valid Admin **200**; exhausted machine source **429**, অন্য source **200**, একই source-এর Admin **200**; machine CPU slots full হলেও Admin verification সফল |
| F4: plain-text framework errors | JSON error normalization, status/Allow/Retry-After বজায় রাখা; outer correlation/no-store | JSON syntax/schema, media type, body size, unknown route/method, path ও query rejection tests পাস |
| F5: untyped response contract | Public OpenAPI-তে typed extensible Search/FareRules/Reprice/Book/PNR/Ticket/Report/Pricing/Wallet models, pending/error variants, units ও headers | 25 commercial paths-এর response schema typed; সব local schema reference resolves; public contract-এ private Admin schemas নেই; actual UAT captures schema দিয়ে যাচাই |
| F6: stale integration docs | নতুন canonical client guide; gross/payable, active held-ticket Issue, disabled real Direct Issue/cancel, casing, replay ও `202` semantics স্পষ্ট | Source ও captured responses-এর সঙ্গে documentation মিলানো হয়েছে |
| F7: Search/Book passenger limit mismatch | Search-ও Book-এর মতো infants-সহ সর্বোচ্চ নয়জন গ্রহণ করে | 8 ADT + 1 INF গ্রহণ; 9 ADT + 1 INF reject; আগের Search-only larger groups আর late Book failure-এ পৌঁছাবে না |

Reprice এখনও supplier read চলাকালে offer/version transaction রাখে; connection use admission দিয়ে bounded করা হয়েছে। এতে acceptance/Book-এর pricing-version serialization বজায় থাকে। এটি distributed admission বা সব endpoint-এর connection isolation-এর দাবি নয়। `DB_MAX_CONNECTIONS` অন্তত 2 প্রয়োজন।

Book-এর gate durable reservation-এর আগে current client authority নিশ্চিত করে। আগে থেকেই committed supplier intent পরের revoke দিয়ে বাতিল হয় না। Credential authentication request entry-তেই হয়; এই fix credential revocation-কে in-flight cancellation বানায় না।

## পরীক্ষা

- `cargo test --locked`: **100 passed, 0 failed, 35 ignored**। Ignored opt-in tests আলাদা environment ছাড়া স্বয়ংক্রিয়ভাবে চালানো হয়নি।
- Fresh database integration suite: **passed**; authentication, search/reprice, Book, ticket/cancellation/reconciliation, tier, managed API, portal, passengers/holds, wallet lock order ও migration constraints অন্তর্ভুক্ত।
- Fresh wallet suite: **passed**; concurrent overspend, replay, immutable ledger, scoped reads, portal roles, report pagination এবং restricted runtime role।
- Dedicated security regression probe: **passed**; F1–F4-এর controlled interleavings/source buckets এবং public-route authentication।
- Final foundation suite: **7 passed**; OpenAPI references, framework errors, headers ও compression।
- Formatting, Clippy all-targets with warnings denied এবং diff whitespace checks চালানো হয়েছে; চূড়ান্ত ফল [verification JSON](CLIENT_API_REMEDIATION_VERIFICATION_2026-09-18.json)-এ।

Tests: [audit/regression probe](../../tests/client_api_audit.rs), [foundation](../../tests/foundation.rs), [private-capture validator](../../scripts/validate-client-contract.py)। Probe-এ নতুন disposable `client_audit_*_test` database, explicit UAT opt-in এবং exact Triplover UAT host assertions প্রয়োজন। প্রতি journey-তে সর্বোচ্চ এক supplier Book ও এক Issue dispatch করা যায়। Raw captures `.local`-এ restricted permissions-এ থাকে; repository report-এ credentials, raw passenger data বা ticket numbers নেই।

## UAT ও contract verification

এই follow-up-এ two fresh isolated UAT journeys চালানো হয়েছে, flight date **18 October 2026** (30 দিন পরে), return 21 October। দুটিতেই:

- One-way Search **200 / 9 offers**; return Search **200 / 90 offers**। FareRules, Reprice, acceptance ও pricing reads **200**।
- One supplier Book dispatch, same-key replay-এ **zero additional dispatch**। Book `202 outcome_unknown` হওয়ায় Issue পাঠানো হয়নি। দুই follow-up run মিলিয়ে **2 Book dispatches, 0 Issue dispatches, 0 wallet ledger entries**।
- Open-jaw multicity Search (DAC→CGP, DAC→CXB) **503 ALL_SUPPLIERS_FAILED**। আগের baseline-এ BG connected multicity ও BS family return-এর সফল read evidence বর্তমান contract দিয়ে পুনরায় validate করা হয়েছে; এই follow-up-এ নতুনভাবে ওই দুই সফল alternative-এর live revalidation দাবি করা হচ্ছে না।

| Attempt | কারণ ও evidence | বর্তমান handling |
| --- | --- | --- |
| `5fb0c02b-3dd8-4157-b6ba-f3d20e837d07` | আগের audit-এর একই synthetic traveller/flight; saved supplier response `isSuccess=false`, explicit duplicate-booking message | Original reservation preserved; no additional dispatch or Issue. New classifier identifies `SUPPLIER_DUPLICATE_REPORTED` without passenger text. |
| `d9fdfa0d-46a5-40be-82c3-295514194ede` | নতুন synthetic traveller; usable Book response absent at **60.022 s** | Original reservation preserved; no retry/Issue/debit. Subsequent read-only supplier lookup found the matching failed operation. |

### Repeated 202 investigation

User-এর repeated `202 outcome_unknown` concern-এর পরে supplier-এর [published UAT OpenAPI](https://userapi-uat.triplover.com/swagger/v1/swagger.json) থেকে দুটি read endpoint যাচাই করা হয়েছে। কেবল নিজের saved supplier transaction ব্যবহার করা হয়েছে:

1. `GET /api/TicketReletedReport/GlobalSearchB2B/{transaction}`: **200**, এক matching record, `status=Failed`।
2. `GET /api/B2BReport/AirTicketingDetails/{transaction}/Confirmed`: **200**, `status=Failed`, `statusFor=Booking`, `isCompleted=true`; কোনো PNR বা ticket number নেই। Transaction, passenger count ও full saved synthetic passenger identity মিলেছে। URL-এর `Confirmed` parameter response success/issuance প্রমাণ করে না।
3. Supplier report-এর `statusReason` বলে তাদের **নিজস্ব internal HTTP call 00:01:00-এ timeout** করেছে। Internal service address, passenger data ও upstream references public report থেকে বাদ রাখা হয়েছে। শুধুমাত্র আমাদের response deadline বাড়ানো এই upstream failure সারাবে না।

**F8 — release blocker remains:** supplier UAT booking timeout/reliability এই code change দিয়ে ঠিক হয়েছে বলে দাবি করা হচ্ছে না। Supplier-side Failed/no-PNR evidence নিজে থেকে airline reservation না হওয়ার চূড়ান্ত নিশ্চয়তা নয়; record জোর করে successful/failed হিসেবে close করা হয়নি। Private supplier case প্রস্তুত আছে; operator/supplier confirmation প্রয়োজন। নতুন successful post-fix Issue যাচাই হয়নি। আগের baseline-এর বাস্তব successful Issue evidence আলাদা historical evidence।

API-side diagnostic gap ঠিক করা হয়েছে: future Book attempts persist safe timeout/transport/authentication/response/business-failure codes; Admin detail exposes `failureCode`. Pending response-এর `reason`, `nextAction`, `statusUrl`, `automaticRetryAllowed:false` দিয়ে client processing বনাম completed-but-unresolved outcome আলাদা করতে পারে। `contact_support` হলে polling বন্ধ করতে বলা হয়েছে; পাঁচ মিনিটের বেশি stale pending-ও support action দেয়। Existing history-তে অনুমান করে timeout code backfill করা হয়নি। Unit tests এবং fresh full database suite timeout বনাম supplier failure, replay, passenger-text redaction ও unchanged reservation safety যাচাই করেছে।

### Schema ও অর্থের যাচাই

**54 captured public responses** served OpenAPI-এর schemas দিয়ে validate হয়েছে; **396 nested fare-breakdown snapshots**-এ exact decimal payable equation ও passenger-count-weighted totals মিলেছে। এগুলো distinct bookings নয়; Search offers, nested receipts এবং replay-সহ snapshots। Raw supplier responses এই count-এ নেই। Scope-এ baseline successful Book/Issue/report/PNR এবং follow-up Search/Reprice/acceptance/pricing/unknown outcomes আছে। Wallet behavior আলাদা integration suite-এ যাচাই হয়েছে; এই follow-up-এ successful live Issue হয়নি, তাই নতুন live wallet debit দাবি করা হচ্ছে না।

Current client guide এবং OpenAPI নতুন diagnostics বর্ণনা করে। Test captures are compatibility evidence, not proof that every supplier extension or itinerary is supported.

## Deployment ও অবশিষ্ট সীমা

- Local Nginx deployment-এ `AUTH_TRUSTED_PROXY_IPS=127.0.0.1,::1` এবং proxy-overwritten `X-Real-IP` প্রয়োজন। Default ফাঁকা হলে peer socket IP ব্যবহৃত হয়; proxy-এর সব caller তার source quota ভাগ করে। [Setup](../SERVER_SETUP.md) অনুসরণ করে target-এ verify করতে হবে। Caller-provided forwarding header সরাসরি বিশ্বাস করা হয় না।
- Backend errors JSON হলেও Nginx/edge-generated errors ভিন্ন হতে পারে। Client HTTP status ও non-JSON transport failures সামলাবে।
- Real Direct Issue ও real held cancellation এখনও disabled। Branded/ancillary/multiple-component সীমা অপরিবর্তিত। এই কাজ সেগুলোকে production-ready ঘোষণা করে না।
- Firsttrip/Takeoff live supplier acceptance, actual production proxy/rate behavior, load capacity, dependency advisory scan এবং infrastructure penetration testing এই verification-এর বাইরে।
- এই পরিবর্তন deployment-ready review-এর জন্য local-এ আছে; target build/configuration deploy ও smoke verify হওয়ার আগে clients-কে live fix পাওয়া গেছে বলা যাবে না।

Client handoff: [CLIENT_API_GUIDE.md](../CLIENT_API_GUIDE.md) এবং running environment-এর `/openapi.json`।

দুই follow-up UAT database-এর private archive ও archive listing যাচাই করা হয়েছে; restore rehearsal দাবি করা হচ্ছে না। Test cluster বন্ধ করা হয়েছে; private evidence ও archives retained আছে।
