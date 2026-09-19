# Client API readiness audit — 18 September 2026

**Historical baseline:** নিচের findings fixes-এর আগের build-এর। পরবর্তী পরিবর্তন, regression tests ও UAT ফলের জন্য [remediation report](CLIENT_API_REMEDIATION_2026-09-18.md) দেখুন। এই baseline evidence অপরিবর্তিত রাখা হয়েছে।

**রায়: মূল UAT Search → Book → Issue flow কাজ করছে। তবে নিচের তিনটি security/availability finding ঠিক না করে unrestricted client rollout করা উচিত নয়।** এছাড়া error contract ও OpenAPI response schema সম্পূর্ণ করা প্রয়োজন। এই audit-এ business implementation পরিবর্তন করা হয়নি।

Reviewed source: `2d2c21f7ed346c906ddbe6f42267f83fa8335225`। নতুন audit probe: [client_api_audit.rs](/Users/ashifbabu/Projects/shapontravels/tests/client_api_audit.rs)।

## Scope ও পরীক্ষার সীমা

- বর্তমান Rust/Axum commercial router, request deserialization, authentication/authorization, ownership, pricing, booking, issue, saved receipt, PNR, ticket report, wallet, idempotency এবং public OpenAPI পরীক্ষা করা হয়েছে। HTTP request/response test সরাসরি আসল Axum router ব্যবহার করেছে; supplier UAT calls বাস্তব HTTPS ছিল।
- আলাদা PostgreSQL cluster ও নতুন disposable databases ব্যবহার করা হয়েছে: `.local/client-api-audit-20260918/`। Working/production database, supplier configuration ও production deployment পরিবর্তন করা হয়নি।
- Triplover UAT-এ **একটি বাস্তব Book এবং একটি বাস্তব NewTicket** পাঠানো হয়েছে। Retry/replay-এ দ্বিতীয় supplier mutation হয়নি। Passenger ও wallet funding ছিল isolated UAT test data।
- Live flight dates শুরু হয়েছে **18 October 2026**, audit day-এর 30 দিন পরে; বিকল্পগুলি আরও পরে। User-এর অন্তত 15 দিন পরের flight শর্ত মানা হয়েছে। Offline fixture tests-এর পুরোনো dates supplier-এ পাঠানো হয়নি।
- Firsttrip/Takeoff-এর live account, production HTTPS/reverse proxy, infrastructure penetration testing, dependency-advisory scan এবং পুরো portal identity rollout এই audit-এর scope-এ যাচাই করা হয়নি। এগুলোর security clearance দাবি করা হচ্ছে না।

## Confirmed findings

### F1 — P1: permission revoke-এর পর queued Book নতুন supplier dispatch করতে পারে

**Affected:** `POST /api/Book`।

`Machine` extractor request শুরুতে `booking` permission পড়ে। এরপর booking transaction authority barrier/client lock-এর জন্য অপেক্ষা করতে পারে। Lock পাওয়ার পরে ordinary machine Book path client-এর বর্তমান permission পুনরায় পরীক্ষা করে না; শুধু row lock নেয়। ফলে revoke transaction commit হওয়ার পরও আগে authenticated request নতুন booking reserve ও dispatch করে।

**Reproduction:** disposable database-এ `booking` permission removal transaction খোলা রাখা হয়; HTTP Book পুরোনো committed permission দিয়ে authenticate করে authority barrier-এ অপেক্ষা করে। Removal commit করলে request resume করে। Recorded result: **HTTP 202, mock supplier Book calls = 1**, যদিও committed permissions তখন শুধু `search:read`। Mock ইচ্ছাকৃতভাবে timeout ফেরায়; finding হলো অনুমতি চলে যাওয়ার পরে dispatch হয়েছে। কোনো live supplier-এ এই race exercise করা হয়নি।

এটি ইতিমধ্যে supplier-এ পাঠানো booking cancel করার দাবি নয়; permission removal commit-এর সময় এই request এখনও নতুন dispatch reservation তৈরি করেনি। নতুন request revoke-এর পরে শুরু করলে স্বাভাবিক permission checks কার্যকর থাকে।

**Source:** [booking.rs:401](/Users/ashifbabu/Projects/shapontravels/src/booking.rs:401), [booking.rs:421](/Users/ashifbabu/Projects/shapontravels/src/booking.rs:421), [identity/business.rs:89](/Users/ashifbabu/Projects/shapontravels/src/identity/business.rs:89)। Held Issue path ইতিমধ্যে lock-এর ভেতরে active/permission পুনরায় পরীক্ষা করে: [ticketing.rs:365](/Users/ashifbabu/Projects/shapontravels/src/booking/ticketing.rs:365)।

**Fix direction:** mutation reservation transaction-এ authority barrier ও client lock পাওয়ার পরে বর্তমান active state, permission এবং managed API eligibility revalidate করতে হবে। Token/credential revocation-এর in-flight semantics-ও একই জায়গায় স্পষ্ট করতে হবে। Queued Book + permission removal regression test যোগ করতে হবে।

### F2 — P1: concurrent RePrice shared database pool আটকে দিতে পারে

**Affected:** `POST /api/Reprice`; একই pool ব্যবহারকারী অন্য client/API-ও প্রভাবিত হয়।

RePrice transaction ও offer row lock নিয়ে supplier response-এর জন্য অপেক্ষা করে। একই offer-এর দ্বিতীয় request আরেকটি database connection নিয়ে সেই lock-এর জন্য অপেক্ষা করে। Search-এর মতো RePrice admission/concurrency guard নেই। SQL lock wait-ও supplier request timeout দিয়ে bounded নয়।

**Reproduction:** দুই-connection pool-এ প্রথম RePrice mock supplier-এ অপেক্ষা করে; দ্বিতীয় RePrice একই offer-এর row lock-এ আটকে থাকে। তখন একটি valid `/auth/me` request **503 `DATABASE_UNAVAILABLE`** পায়। Test শেষে held request release করলে স্বাভাবিক response ফেরে। Default pool বড় হলে বেশি concurrent request দরকার; দুর্বলতাটি pool size-এর সঙ্গে scale করে।

**Source:** [reprice.rs:65](/Users/ashifbabu/Projects/shapontravels/src/reprice.rs:65), [reprice.rs:106](/Users/ashifbabu/Projects/shapontravels/src/reprice.rs:106)।

**Fix direction:** database connection নেওয়ার আগে per-client/global RePrice admission ও bounded queue; duplicate selection-এর concurrent calls coalesce বা দ্রুত reject করা; short transaction/version reservation দিয়ে supplier I/O থেকে long-held database transaction সরানো। Multi-instance deployment হলে coordination policy নির্দিষ্ট করতে হবে।

### F3 — P1: unauthenticated requests দিয়ে সব account-এর login/token exchange বন্ধ করা যায়

**Affected:** `POST /auth/token`, `POST /admin/login`।

দুই endpoint একই `auth:global` 120/minute bucket ব্যবহার করে। Token request-এর secret length reject হওয়ার **আগে** global quota consume হয়। Random UUID + 257-character invalid secret দিয়ে password hashing ছাড়াই quota শেষ করা যায়। কোনো valid credential প্রয়োজন নেই।

**Reproduction:** test Admin-এর correct credentials দিয়ে login প্রথমে **200**। এরপর 120টি unauthenticated oversized-secret token request। সেই একই correct Admin login এবার **429 `RATE_LIMITED`**। প্রতি window-তে এটি পুনরাবৃত্তি করে token renewal ও Admin login ব্যাহত করা সম্ভব। Existing valid machine tokens সরাসরি revoke হয় না।

**Source:** [auth.rs:113](/Users/ashifbabu/Projects/shapontravels/src/auth.rs:113), [auth.rs:272](/Users/ashifbabu/Projects/shapontravels/src/auth.rs:272)। Repository-এর Nginx example-এ source-specific login limiter নেই: [SERVER_SETUP.md:287](/Users/ashifbabu/Projects/shapontravels/docs/SERVER_SETUP.md:287)। Deployed proxy-এর actual protections এই audit-এ পরীক্ষা করা হয়নি।

**Fix direction:** malformed input আগে reject; trusted proxy/peer identity ভিত্তিক source limiter; human login ও machine exchange-এর পৃথক capacity; global circuit breaker এমনভাবে বসানো যাতে একটি unauthenticated source অন্যদের capacity শেষ করতে না পারে। শুধু 120 limit বাড়ানো বা শুধু length validation সরানো যথেষ্ট নয়।

### F4 — P2: validation errors সবসময় JSON নয়

`POST /api/Search`-এ valid token সহ `{}` পাঠালে **422 plain text** পাওয়া গেছে:

```text
Failed to deserialize the JSON body into the target type: missing field `routes` at line 1 column 2
```

Business errors একই API-তে `{"error":"CODE"}` JSON। ফলে client SDK সব error-এ `response.json()` করলে নিজস্ব parsing exception পাবে এবং আসল validation error হারাবে। Authentication documentation এই Axum exception উল্লেখ করে; এটি নতুন তথ্যফাঁস হিসেবে report করা হচ্ছে না, কিন্তু external client contract অসমান।

**Source:** [search.rs:330](/Users/ashifbabu/Projects/shapontravels/src/search.rs:330), [auth.rs:22](/Users/ashifbabu/Projects/shapontravels/src/auth.rs:22), [AUTHENTICATION.md](/Users/ashifbabu/Projects/shapontravels/docs/AUTHENTICATION.md)।

**Fix direction:** JSON/Path/Query rejection ও unsupported-content-type/body-size errors-এর জন্য documented JSON envelope, stable code, optional field details এবং request ID বজায় রাখা।

### F5 — P2: OpenAPI দিয়ে complete client response contract পাওয়া যায় না

Search, FareRules, Reprice, Book, NewTicket এবং অনেক servicing response `body=Object` হিসেবে প্রকাশ করা হচ্ছে। ফলে generated SDK মূল response fields, `200` বনাম `202` outcome, accepted payable, ticket details, nullable/omitted fare fields এবং error codes নির্ভরযোগ্যভাবে টাইপ করতে পারে না। Runtime validation rules-এর অনেকগুলোও schema-তে প্রকাশিত নয়।

**Source:** [search.rs:329](/Users/ashifbabu/Projects/shapontravels/src/search.rs:329), [reprice.rs:56](/Users/ashifbabu/Projects/shapontravels/src/reprice.rs:56), [booking.rs:384](/Users/ashifbabu/Projects/shapontravels/src/booking.rs:384), [ticketing.rs:267](/Users/ashifbabu/Projects/shapontravels/src/booking/ticketing.rs:267)।

**Fix direction:** versioned response schemas, success/pending/error examples, currency/amount units, reference casing, required headers ও nullable fields প্রকাশ করা। Supplier-compatible extensibility প্রয়োজন হলে known core fields-এর সঙ্গে explicitly allowed additional properties রাখা যায়।

### F6 — P2: প্রধান client guide-এর pricing/capability description পুরোনো ও পরস্পরবিরোধী

[SEARCH_API.md](/Users/ashifbabu/Projects/shapontravels/docs/SEARCH_API.md)-এর “Response and pricing” অংশে `totalPrice`-কে markup-সহ selling total বলা হয়েছে; একই guide-এর limits-এ NewTicket/ticket issue unavailable বলা আছে। অথচ current B2B implementation-এ `totalPrice` published gross এবং final payable আলাদা; এই audit-এ NewTicket বাস্তব UAT-এ 200 দিয়েছে। নতুন [B2B_TIERS.md](/Users/ashifbabu/Projects/shapontravels/docs/B2B_TIERS.md) ও [TICKETING_API.md](/Users/ashifbabu/Projects/shapontravels/docs/TICKETING_API.md) সঠিক নতুন নিয়ম দেয়, কিন্তু primary endpoint guide superseded অংশগুলোকে স্পষ্টভাবে চিহ্নিত করে না।

**Impact:** client পুরোনো Search guide মেনে `totalPrice` final charge ধরতে পারে, অথবা supported Hold Issue বাদ দিতে পারে। এটি pricing calculation-এর bug নয়; client-facing specification contradiction।

**Fix direction:** একটিমাত্র current integration guide/version রাখুন; B2B বনাম B2C pricing semantics, payable field, capability matrix ও historical/deployed/local status একসঙ্গে মিলিয়ে দিন।

## Live UAT evidence

| Scenario | Result |
| --- | --- |
| DAC → CXB, 18 Oct 2026, 1 ADT | Search 200; 9 offers |
| Selected one-way FareRules / Reprice / Accept | 200 / 200 / 200 |
| Hold Book / same-key replay | 200 / 200; equal response; one upstream Book |
| NewTicket / same-key replay | 200 / 200; equal response; one upstream Issue |
| Saved booking / saved ticket / ticket report | 200 / 200 / 200 |
| Explicit PNR read after Issue | 200; supplier status `Ticketed` |
| DAC → CXB → DAC, 18–21 Oct 2026 | Search 200; 90 offers; FareRules/Reprice/Accept all 200 |
| Initial open-jaw multicity: DAC → CGP, then DAC → CXB, 18–21 Oct | Search 503 `ALL_SUPPLIERS_FAILED`; additional routes tested separately |
| BG multicity: DAC → CGP → DAC → CXB, 23/26/28 Oct | Search 200; 3 offers; FareRules/Reprice/Accept all 200 |
| TG multicity: DAC → BKK → SIN, 23–26 Oct | Supplier adapter timeout at 60 seconds; public Search 503 `ALL_SUPPLIERS_FAILED` |
| BS family return: DAC → CXB → DAC, 23–26 Oct; 2 ADT + 1 child age 6 + 1 INF | Search 200; 72 offers; FareRules/Reprice/Accept all 200 |

The successful UAT booking ID is `046e0104-8a29-4ada-9522-18ff49b8d6c0`. One verified passenger ticket was returned. Ticket numbers and raw supplier/passenger data are retained only under the ignored private evidence directory.

**Wallet reconciliation:** accepted payable **BDT 5,658.00**; one captured wallet operation **565,800 minor units**; final held balance **0**. Synthetic initial balance was BDT 1,000,000.00 and final available balance BDT 994,342.00. No second debit occurred on Issue replay.

**Arithmetic verification:** 184 returned Search/Reprice/receipt fare breakdowns passed both `baseFare + taxes + ait + serviceCharge − discount = payable` and passenger-count-weighted aggregation for every monetary component. Counts include repeated receipt snapshots, not 184 unique flights.

Initial multicity failure-এর raw upstream cause প্রথম run-এ capture হয়নি। Alternative TG failure-এ adapter `Timeout` capture হয়েছে। BG multicity success দেখায় route availability/latency নির্ভরতা আছে; এই failures থেকে সব multicity broken অথবা backend pricing bug বলা যাচ্ছে না। Configured UAT timeout ছিল 60 seconds; 120-second budget দিয়ে এই route retest করা হয়নি।

Sanitized machine-readable results: [CLIENT_API_AUDIT_VERIFICATION_2026-09-18.json](/Users/ashifbabu/Projects/shapontravels/docs/evidence/CLIENT_API_AUDIT_VERIFICATION_2026-09-18.json)। Raw responses, logs এবং UAT booking database backup `.local/client-api-audit-20260918/`-এ private অবস্থায় রাখা হয়েছে।

## Important integration differences — intentional, not confirmed bugs

1. **Published gross ≠ final payable.** এই UAT ticket-এর legacy `totalPrice`/report `ticketingPrice` **5,349.00**, কিন্তু accepted `fareBreakdown.payable` ও wallet debit **5,658.00**। বর্তমান B2B pricing policy অনুযায়ী এটি intentional; হিসাব ভুল পাওয়া যায়নি। Final charge-এর জন্য `fareBreakdown.payable` বা `/api/pricing/booking/{id}` ব্যবহার করতে হবে। Report-এর legacy gross দিয়ে invoice/debit reconcile করা যাবে না। [B2B_TIERS.md](/Users/ashifbabu/Projects/shapontravels/docs/B2B_TIERS.md), [FARE_BREAKDOWN_API.md](/Users/ashifbabu/Projects/shapontravels/docs/FARE_BREAKDOWN_API.md)।
2. **Fare breakdown shape endpoint অনুযায়ী আলাদা।** Search/Reprice/Book/NewTicket-এ reconciled `fareBreakdown` object; ticket report-এ existing passenger-type `fareBreakdown` array। একই DTO দিয়ে দুটো parse করা যাবে না। Report endpoint-এ `item1/item2` envelope-ও নেই।
3. **Saved Book response live booking status নয়।** Issue-এর পরও original Book receipt-এর `bookingStatus=Created` থাকা expected; `/ticket`, `X-Ticket-State` এবং explicit PNR থেকে ticket status নিতে হবে।
4. **Same key = replay; new key = new booking intent.** একই quote/passenger দিয়েও নতুন key নতুন Hold করতে পারে, আগের outcome unknown হলেও। এটি documented policy ও existing test-এ intentional। Retry-তে কখনও key regenerate করা যাবে না। [BOOKING_API.md](/Users/ashifbabu/Projects/shapontravels/docs/BOOKING_API.md)।
5. **Live capability সীমা:** normal real adapter-এ Direct Issue এবং সাধারণ Cancel enablement নেই; documented offline/explicit UAT-only restrictions আছে। এগুলোকে unrestricted client capability হিসেবে advertise করা উচিত নয়।

## Verification coverage

- Existing regular suite: **94 passed, 0 failed, 32 ignored**। Ignored suites-কে passed হিসেবে ধরা হয়নি।
- Explicit disposable database integration: **passed**; authentication lifecycle, client ownership, quote/acceptance, Book/Issue concurrency/idempotency, bad supplier evidence, cancellation/reconciliation, pricing and constraints assertions অন্তর্ভুক্ত।
- Explicit wallet integration: **passed**; overspend/concurrency, replay, freeze, immutable ledger, owner scope, portal financial checks, runtime DB-role permissions।
- New offline audit probe: **passed**, অর্থাৎ F1–F3 reproducible এবং F4 observed। এই test দুর্বলতার বর্তমান আচরণ assert করে; এটি remediation acceptance test নয়।
- Public commercial OpenAPI-তে **25 paths**; listed commercial methods-এ unauthenticated calls 401, machine token দিয়ে Admin OpenAPI 401। Existing integration-এর foreign-owner assertions passed; tested paths-এ cross-client data disclosure পাওয়া যায়নি।
- UAT journey probe completed; per-operation statuses উপরের table-এ। Probe completion মানে প্রতিটি scenario সফল নয়—initial multicity failure আলাদাভাবে দেখানো হয়েছে।
- Alternate UAT probe completed; দ্বিতীয় Book/Issue পাঠানো হয়নি। Formatting, targeted Clippy with warnings denied এবং diff checks passed।

## Release recommendation

প্রথমে F1–F3 fix করে controlled concurrency/revocation regressions চালানো উচিত। Client onboarding-এর আগে F4–F6 এবং amount/status/idempotency guide একসঙ্গে publish করা উচিত। UAT success বর্তমান source ও tested Triplover routes-এর evidence; production supplier/account, deployed build ও reverse-proxy configuration-এর acceptance আলাদাভাবে করতে হবে।

Reproduction commands use a **new, empty, loopback database** per probe:

```sh
CLIENT_AUDIT_DATABASE_URL='<new client_audit_*_test database>' \
  cargo test --locked --test client_api_audit offline_security -- --ignored --nocapture

# Read-only UAT alternatives. Supply a new private evidence directory too.
UAT_CLIENT_AUDIT=yes CLIENT_AUDIT_UAT_DATABASE_URL='<new client_audit_*_test database>' \
  CLIENT_AUDIT_EVIDENCE_DIR='<new private directory>' \
  cargo test --locked --test client_api_audit uat_alternate_search -- --ignored --nocapture
```

`uat_client_journey_audit` performs a real UAT Book and Issue and therefore must not be casually rerun as a read-only check. Tests validate exact Triplover UAT hosts, require a fresh database, and cap each mutation at one dispatch.
