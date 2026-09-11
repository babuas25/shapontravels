# Flight Aggregation & Booking API — Requirements

Status: Implementation in progress — Steps 1–2 verified complete; Step 5 implemented within documented coverage; Steps 3–4 and 6 partially implemented; Step 7 partially implemented (held-ticket UAT); Step 8 partially verified. Latest prebooking/release verification update: 2026-09-10. Historical milestone notes below are superseded where later updates say so.

Important current priority (2026-09-10): আরও optimization প্রয়োজন, কোনো valid offer/field বাদ দিয়ে নয়। See [Tripfeels source comparison](docs/evidence/TRIPFEELS_OPTIMIZATION_COMPARISON_2026-09-10.md).

### Current completion checklist — 2026-09-11

এটি বর্তমান status-এর সংক্ষিপ্ত তালিকা। নিচের dated logs ঐতিহাসিক evidence; পরবর্তী implementation/approval আগের pending note-কে supersede করে। একটি বড় Step-এর কিছু অংশ বাকি থাকলে পুরো Step unchecked থাকবে। Local/UAT completion production deployment বোঝায় না।

- [x] Rust/API foundation, machine/client authentication ও পৃথক human Admin authentication।
- [x] Supplier adapters, captured fixtures, public Search/FareRules, markup Admin APIs এবং documented coverage-এর pricing/selection implementation; broader supplier/equivalence coverage এখনও partial।
- [x] Public RePrice, selected directions, latest-price acceptance ও reference ownership।
- [x] Approved return/multicity markup: প্রথম requested route + সেই route-এর প্রথম segment airline; whole-journey passenger fare-এ একবার markup।
- [x] Branded content UAT verification; supplier-এর `null` অপরিবর্তিত রাখা।
- [x] Hold booking, durable idempotency, saved retrieval, live PNR lookup ও reconciliation evidence/deadline persistence।
- [x] Coding-free Bengali Admin reconciliation screen, audited manual resolution এবং browser verification।
- [x] Representative BS return UAT Hold → PNR সফল; BS one-way supplier Hold/PNR পাওয়া গেলেও সংরক্ষিত unknown outcome-এর resolution বাকি।
- [x] BG three-route multicity UAT Hold ও retrieval সফল; supplier status Created এবং deadline খালি—ticket issue readiness এখনও প্রতিষ্ঠিত নয়।
- [x] `STR` + GDS PNR + airline PNR public reference, owner-scoped reference retrieval, Admin display; `STR8FE94RKECOCE` দিয়ে retained UAT booking HTTP 200 যাচাই।
- [x] Latest reference work: ordinary tests, PostgreSQL integration, real saved UAT retrieval, Clippy, formatting ও JavaScript syntax checks pass।
- [x] Held-booking NewTicket implemented; Triplover UAT BS return confirmed with two tickets and saved retrieval HTTP 200।
- [ ] Direct Issue (latest user instruction excludes it), reports এবং production commercial controls/verification।
- [ ] Cancel flow (user-deferred), unresolved booking cases এবং full lifecycle/supplier acceptance।
- [ ] Real populated branded/multiple-component evidence পেলে সেই coverage-এর কাজ (user-deferred)।
- [ ] Broader normalization/equivalence coverage, production private-data protection/retention এবং remaining contract/release acceptance।
- [x] Latest Hold/Admin/markup/reference changes production-এ released (2026-09-11, `bdd3ec7`); migrations 0013–0014 ও deployed readiness verified। Supplier Hold/Issue validation UAT-only থাকে।

### Latest execution environment instruction — 2026-09-10

- User prohibits Hold/Book and Issue through production supplier APIs. Booking validation must use Triplover UAT only; the user has updated the local `.env` for this purpose. This restriction supersedes historical production mutation permissions.
- Local configuration inspection confirms Triplover Search host `searchapi-uat.triplover.com` and servicing host `userapi-uat.triplover.com`, matching the supplied UAT contract. Credentials are present; this inspection does not verify authentication or a running/deployed process's configuration.
- Before any booking execution, assert both UAT hosts and use fresh UAT Search/RePrice references. Do not reuse production references or fall back to a production supplier. An enabled ticketing configuration flag alone is not an instruction to issue a ticket.

## 1. উদ্দেশ্য ও scope

Rust backend সরাসরি Firsttrip, Takeoff ও Triplover-এর তিনটি connection ব্যবহার করবে। Supplier response একটি internal canonical model-এ normalize করে আমাদের frontend এবং external API clients-কে supplier-compatible request/response shape-এ API দেবে। আমাদের নিজস্ব PostgreSQL database থাকবে।

Requirements baseline-এর পর 2026-09-08-এ ব্যবহারকারী step-by-step implementation এবং verified completion tracking অনুমোদন করেছেন। Supplier live/UAT validation ও বাস্তব booking/ticketing execution পৃথক অনুমোদনসাপেক্ষ।

## 2. Technology stack

- Rust + Axum: HTTP API এবং application services।
- PostgreSQL: আমাদের project-এর নিজস্ব database; পুরোনো Node.js application database-এর ওপর dependency থাকবে না।
- Versioned database migrations।
- OpenAPI specification ও Swagger UI: technical documentation এবং authenticated interactive testing।
- নির্দিষ্ট crate, deployment provider ও infrastructure implementation design-এ নির্ধারিত হবে।

## 3. Admin & Authentication — Final Decision (2026-09-08)

- B2B API clients Client ID + Client Secret ব্যবহার করবে। Credentials exchange করে short-lived opaque Bearer access token পাবে; resource API-তে client secret নয়, access token যাবে।
- Access token lifetime ঠিক 30 minutes (1,800 seconds)। Expiry হলে client machine-to-machine credentials exchange দিয়ে স্বয়ংক্রিয়ভাবে নতুন token নেবে; human re-login বা initial version-এ refresh token লাগবে না। Automatic replacement client integration/SDK-এর দায়িত্ব হিসেবে document করতে হবে।
- Client disable এবং credential revocation/reset existing authorization invalidate করবে; শুধু future token issuance বন্ধ করাই যথেষ্ট নয়। Resource validation-এ active client/credential status ও revocation enforce করতে হবে।
- Client Secret শুধু issue/reset-এর সময় দেখানো হবে; secure verification hash সংরক্ষণ হবে, plaintext secret logs/database-এ নয়। Rotation/revocation support এবং audit থাকবে। Rotation-এ retired/revoked credential-এর authorization policy reset/revocation guarantee bypass করতে পারবে না।
- Human Admin/Super Admin authentication B2B machine authentication থেকে পৃথক থাকবে। B2B token দিয়ে human administrative privileges পাওয়া যাবে না। Supplier authenticationও আলাদা; supplier credentials/token client-কে দেওয়া যাবে না।
- Human admin credentials/sessions দিয়ে agencies commercial Flight API call করতে পারবে না; commercial endpoints machine access token চাইবে। দুই security context পরস্পরের credential/session গ্রহণ করবে না।
- শুধু authorized Admin/Super Admin markup rules create/edit/activate/deactivate/manage করতে পারবে। Agent/API client নিজের markup কোনোভাবেই modify করতে পারবে না।
- Authorized Admin/Super Admin API clients provision ও lifecycle manage করবে। Admin প্রতিটি API client-এর commercial pricing context নির্ধারণ করবে: B2B/B2C context এবং applicable Specific Agent pricing override। Client audience/agent assignment trusted administrative configuration থেকে হবে।
- Public admin registration বা default admin password থাকবে না। Initial Super Admin controlled one-time bootstrap দিয়ে তৈরি হবে; bootstrap rerun দিয়ে existing installation-এ নতুন Super Admin তৈরি বা credentials overwrite করা যাবে না।
- Bootstrap-এর পরে administrator management শুধু authorized administrative access দিয়ে হবে।
- Client permissions: search/read, booking, cancellation ও ticketing। Administrative operations পৃথক protected authentication/authorization boundary-তে থাকবে।
- Client শুধু নিজের search references, bookings ও tickets access করতে পারবে। Per-client rate limits এবং authentication failure protection থাকবে।
- Token exchange endpoint-এর path, wire schema, human admin session mechanism ও bootstrap command details implementation design-এ document হবে; এগুলো উপরের final business/auth decisions পুনরায় খোলার কারণ নয়। পুরোনো email/password supplier-style public client Login requirement বাতিল।

## 4. Supplier connections

- Supported connection identifiers: `firsttrip`, `takeoff`, `triplover`। Shared supplier protocol adapter এবং connection-specific configuration ব্যবহার করতে হবে।
- প্রত্যেক connection-এর `SEARCH_BASE_URL`, `BASE_URL`, `EMAIL`, `PASSWORD` environment/secrets configuration থেকে নিতে হবে। Secret source code, response বা logs-এ প্রকাশ করা যাবে না।
- Search host এবং অন্য endpoint-এর host পৃথকভাবে পরিচালনা করতে হবে।
- প্রত্যেক connection-এর supplier authentication/token cache আলাদা থাকবে। Expiry অনুযায়ী renewal এবং concurrent refresh coordination থাকতে হবে।
- Configuration presence/format validation এবং real credential validity আলাদা status হিসেবে বিবেচনা করতে হবে।
- Connection enable/disable, timeout এবং booking/ticketing capability configuration থাকতে হবে। Supplier account entitlement বাস্তবে যাচাই ছাড়া ধরে নেওয়া যাবে না।
- শুধু enabled supplier connections-এ Search concurrently হবে। একটি active supplier timeout/fail করলে অন্যগুলোর valid result ফেরত দেওয়া যাবে; partial result status supplier-compatible existing field অথবা documented response header দিয়ে জানাতে হবে; নতুন body field নয়। সব active supplier fail করলে পরিষ্কার error দিতে হবে। ইচ্ছাকৃতভাবে disabled supplier-কে failure বা partial-result কারণ হিসেবে গণনা করা যাবে না।

### 4.1 পছন্দমতো supplier চালু/বন্ধ

- Authorized administrator প্রত্যেক supplier স্বাধীনভাবে চালু/বন্ধ করতে পারবে: শুধু Triplover, শুধু Firsttrip, শুধু Takeoff, যেকোনো দুটি অথবা তিনটিই একসঙ্গে।
- এই selection project-wide হবে এবং protected admin API দিয়ে পরিবর্তন করা যাবে। PostgreSQL-এ configuration persist হবে; code change, redeploy বা restart প্রয়োজন হবে না। পরিবর্তনের audit record রাখতে হবে।
- নতুন Search শুরুর সময় active connections-এর snapshot নিতে হবে। Lowest-fare comparison শুধু সেই active connections-এর returned offers-এর মধ্যে হবে। একটি active থাকলে তার সব distinct class/fare option থাকবে।
- সব supplier বন্ধ রাখা যাবে; তখন নতুন Search documented `NO_ACTIVE_SUPPLIERS` business error দেবে, misleading empty success নয়।
- Supplier বন্ধ করার পরে তার পুরোনো unbooked search offer দিয়ে নতুন RePrice/Book করা যাবে না; client-কে নতুন Search করতে হবে। চলমান request cancel হয়েছে এমন দাবি করা যাবে না; accepted/in-flight operation-এর outcome record/reconcile করতে হবে।
- Search participation বন্ধ করা এবং existing booking servicing বন্ধ করা আলাদা control। আগে তৈরি booking-এর PNR, report, eligible Cancel ও NewTicket মূল supplier connection দিয়েই চলবে, existing permissions এবং পৃথক booking/ticketing execution gates সাপেক্ষে। Supplier disable করে existing booking অন্য supplier-এ সরানো যাবে না।
- Supplier credentials revoke বা operational execution block করা হলে servicing unavailable হতে পারে; পরিষ্কার error এবং প্রয়োজনমতো manual resolution status দিতে হবে।

## 5. Canonical model ও lowest-fare নির্বাচন

### 5.1 মূল নিয়ম

**প্রতি itinerary-এর সব আলাদা class/fare option রাখতে হবে। প্রতিটি সমমানের option-এর জন্য active connections-এর সর্বনিম্ন original Supplier total price (সব যাত্রীর মোট, আমাদের markup ছাড়া) দিয়ে supplier নির্বাচন করতে হবে। এরপর নির্বাচিত offer-এ applicable markup যোগ করে user-facing Selling Fare হবে। পুরো itinerary-এর শুধু একটি cheapest class রেখে অন্য class বাদ দেওয়া যাবে না।**

উদাহরণ — একই flight, একই passenger mix ও currency; নিচের দামগুলো original Supplier total price, আমাদের markup ছাড়া:

| Class / fare option | Firsttrip | Takeoff | Triplover | Returned option |
|---|---:|---:|---:|---|
| Economy Saver | 5,500 | 5,300 | 5,400 | 5,300 — Takeoff |
| Economy Flex | 6,200 | 6,400 | 6,100 | 6,100 — Triplover |
| Business | 15,000 | 14,500 | 15,200 | 14,500 — Takeoff |

- তিনটি option-ই response-এ থাকবে। একটি option শুধু এক connection-এ থাকলেও তা রাখতে হবে।
- Flight card-এর starting price প্রয়োজন হলে সব returned option-এর minimum থেকে derive করা যাবে; এতে অন্য option বাদ যাবে না।

### 5.2 Itinerary identity

- Ordered routes এবং সব flight segments ধরে identity নির্ধারণ করতে হবে।
- Origin/destination, departure date/time, marketing/operating carrier, flight number ও connection sequence বিবেচনা করতে হবে।
- One-way, round-trip এবং multicity সমর্থন করতে হবে। একই flight number কিন্তু ভিন্ন date বা routing এক itinerary নয়।
- Round-trip/multicity-এর fare সম্পূর্ণ supplier offer হিসেবে তুলনা হবে। আলাদা supplier-এর সস্তা leg জোড়া দিয়ে নতুন bookable itinerary তৈরি করা যাবে না।

### 5.3 Fare-option equivalence

- Segment-wise `bookingClass`/RBD, `serviceClass`, fare brand/fare basis যেখানে পাওয়া যায়, checked/cabin baggage, refundability এবং প্রাসঙ্গিক change/refund conditions বিবেচনা করতে হবে। User-approved 2026-09-09: class comparison-এর প্রধান field `bookingClass` (যেমন Q/V); `cabinClass` null/missing হলেও অন্য required attributes মিলে গেলে তুলনা হবে। RBD থেকে cabin label অনুমান করা যাবে না। Explicit cabin labels পরস্পর বিরোধী হলে সেই comparison group আলাদা থাকবে।
- শুধু `Economy` label দেখে merge করা যাবে না। একই cabin-এর ভিন্ন RBD বা fare product আলাদা option থাকবে।
- Baggage allowance passenger type ও segment অনুযায়ী normalize করতে হবে। Baggage ছাড়া এবং baggage-সহ fare আলাদা option।
- Supplier brand names আলাদা হলে verified mapping ব্যবহার করতে হবে। Unknown/missing field-কে অনুমান করে equivalent ধরা যাবে না; নিশ্চিত match না হলে আলাদা option রাখতে হবে।
- প্রতিটি canonical offer-এ itinerary, fare attributes, passenger breakdown, monetary totals, currency, source connection, opaque source references ও booking capability সংরক্ষণ করতে হবে।

### 5.4 Price comparison

- একই passenger mix/count, child ages, itinerary ও currency-তে সব যাত্রীর original Supplier total price তুলনা করতে হবে; আমাদের markup বাদ থাকবে, per-adult starting fare নয়।
- Base fare, tax ও mandatory fees অন্তর্ভুক্ত থাকবে। Optional ancillary যোগ করলে একই selection-এর দাম তুলনা করতে হবে। কোনো component double-count করা যাবে না।
- Money calculation exact decimal বা currency minor units দিয়ে করতে হবে; floating-point arithmetic নয়।
- Cross-currency comparison currency-conversion contract ছাড়া করা যাবে না।
- User correction — 2026-09-09: comparable offers-এর original Supplier total price দিয়ে lowest supplier নির্বাচন হবে। এরপর নির্বাচিত original offer-এ §5.5–§5.6 অনুযায়ী applicable markup যোগ করে client-facing Selling Fare হবে। Markup বা Selling Fare supplier ranking বদলাবে না। এটি আগের markup-পরবর্তী lowest-selection নিয়ম প্রতিস্থাপন করে। পুরোনো application-এর অন্য pricing বা business rules স্বয়ংক্রিয়ভাবে inherit করা যাবে না।
- Equal original Supplier total price হলে documented deterministic connection priority দিয়ে tie resolve করতে হবে; markup দিয়ে tie resolve হবে না। User-approved priority (2026-09-09): Takeoff → Firsttrip → Triplover। অন্য equivalent supplier offers internalভাবে রাখা যাবে।
- Search price guaranteed নয়। RePrice-এর পর পরিবর্তিত total ও conditions client-কে জানাতে হবে; পরিবর্তিত দাম গ্রহণ ছাড়া Book/issue চালানো যাবে না।
- RePrice-এর পর silent supplier switching হবে না। অন্য supplier offer বেছে নিলে তার নিজস্ব reference দিয়ে নতুন RePrice ও client acceptance প্রয়োজন।

### 5.5 Final Markup Business Rules & Fallback — 2026-09-07

এই section ব্যবহারকারীর সর্বশেষ final specification। এটি আগের সব markup rules এবং অন্য project থেকে নেওয়া markup baseline সম্পূর্ণ প্রতিস্থাপন করে। উদ্দেশ্য: applicable Supplier Fare-এর ওপর একটি markup নির্ধারণ করে customer/agent-এর Selling Fare তৈরি করা। Scope Search → RePrice → Book → Issue/Confirmed পর্যন্ত।

**মূল নিয়ম:** একটি fare-এর জন্য একটিমাত্র applicable markup rule apply হবে। একাধিক matching markup যোগ হবে না। প্রথম applicable priority match-ই final; তারপর অন্য rules বিবেচনা করা হবে না।

#### Audience

- B2C Customers।
- All B2B Users।
- Specific Agent: B2B-এর special exception। Agent-এর matching rule general B2B rule override করবে; agent-এর কোনো matching rule না থাকলে All B2B rules-এ fallback হবে।

Audience এবং agent identity trusted client/account context থেকে resolve করতে হবে। Arbitrary request field দিয়ে অন্য agent-এর special pricing নেওয়া যাবে না। Audience classification customer login/dashboard তৈরি করার নতুন requirement নয়।

#### Airline/route scopes ও একই audience-এর priority

| Priority | Airline scope | Route scope |
|---|---|---|
| 1 | Specific Airline | Specific Route |
| 2 | All Airlines | Specific Route |
| 3 | Specific Airline | All Routes |
| 4 | All Airlines | All Routes |

Specific route-এর উদাহরণ DAC → SIN। All/All হলো audience-এর সাধারণ/default rule।

#### Specific Agent resolution — audience আগে, scope পরে

ABC Travels-এর জন্য resolution order:

1. ABC + Specific Airline + Specific Route।
2. ABC + All Airlines + Specific Route।
3. ABC + Specific Airline + All Routes।
4. ABC + All Airlines + All Routes।
5. All B2B + Specific Airline + Specific Route।
6. All B2B + All Airlines + Specific Route।
7. All B2B + Specific Airline + All Routes।
8. All B2B + All Airlines + All Routes।

Agent-এর চারটি scope-এর মধ্যে কোনো match পেলেই সেটি final। এমনকি Agent All/All match থাকলেও তা B2B airline/route-specific rule-এর আগে জিতবে। Agent rules-এ কোনো applicable match না থাকলেই B2B-এর চারটি scope evaluate হবে। Agent-এর unrelated special rule থাকার কারণে B2B fallback বন্ধ হবে না।

#### B2C এবং সাধারণ B2B resolution

B2C শুধু নিজের চারটি priority scope evaluate করবে; Specific Agent বা B2B fallback নেই। সাধারণ B2B user নিজের All B2B চারটি scope ব্যবহার করবে। B2B/Agent থেকে B2C fallback নেই।

#### Final business example

| Rule | Audience | Airline | Route | Fixed markup |
|---|---|---|---|---:|
| 1 | All B2B | All | All | ৳500 |
| 2 | All B2B | All | DAC → SIN | ৳400 |
| 3 | ABC Travels | SQ | All | ৳200 |
| 4 | ABC Travels | SQ | DAC → SIN | ৳100 |

| ABC Travels search | Winning rule | Applied markup |
|---|---|---:|
| SQ DAC → SIN | 4 | ৳100 |
| SQ DAC → BKK | 3 | ৳200 |
| BG DAC → SIN | 2, via B2B fallback | ৳400 |
| BG DAC → BKK | 1, via B2B fallback | ৳500 |

#### Calculation

S = supplier original per-passenger `totalPrice`; M = winning rule-এর per-passenger markup; U = new per-passenger `totalPrice`। প্রতিটি passenger type-এর individual fare-এ calculation করে পরে count multiply হবে (§5.6)। একই tax/fee/AIT দ্বিতীয়বার যোগ করা যাবে না।

- **Fixed Amount:** M = configured fixed amount; U = S + M। S=৳30,000 এবং Fixed=৳500 হলে U=৳30,500। Adult-reference proportional formula প্রযোজ্য নয়।
- **Percentage:** M = S × percentage / 100; U = S + M। S=৳30,000 এবং Percentage=2% হলে M=৳600, U=৳30,600।
- Markup applicable supplier fare-এর ওপর হবে। পূর্বে markup করা Selling Fare-এর ওপর আরেকটি markup stage নয়।
- No stacking example: B2B default=500, SQ=400, DAC→SIN=300, SQ+DAC→SIN=200 হলে SQ DAC→SIN-এর markup শুধু 200, কখনো 1,400 নয়।

#### Search থেকে Confirmed-এ প্রয়োগ

- প্রতিটি active supplier-এর comparable class/fare option-এর original Supplier total price তুলনা করে lowest option নির্বাচন হবে। নির্বাচিত offer-এর জন্য audience fallback ও winning markup rule resolve করে Selling Fare হিসাব হবে; অন্যান্য distinct class/fare options বাদ যাবে না।
- Search ও RePrice একই calculation engine ব্যবহার করবে। Selected source supplier/account/references, audience/agent, winning rule/version, supplier fare, calculated markup, Selling Fare ও acceptance snapshot সংরক্ষণ হবে।
- RePrice-এ দাম/terms বদলালে নতুন pricing version এবং client acceptance প্রয়োজন। Book/Issue accepted version ব্যবহার করবে; markup আবার যোগ হবে না। Confirmed record-এ accepted Selling Fare ও verified ticket evidence থাকবে।
- Rule management-এর schemas, audience/scope, Fixed/Percentage examples ও price responses Swagger-এ document করতে হবে। Rule mutation শুধু authorized human Admin/Super Admin করতে পারবে (§3); Agent/API client নয়।

#### Final specification-এ এখনও অনির্ধারিত edge cases

নিচের unresolved বিষয়গুলো speculative সিদ্ধান্ত দিয়ে স্থির করা যাবে না; এগুলো project implementation শুরু করার global blocker নয়। §5.7-এর evidence/approval process অনুসরণ করতে হবে:

- Audience fallback chain-এর শেষেও কোনো match না থাকলে আচরণ; আগের Base+Tax/Gross fallback আর অনুমোদিত নয়।
- দুই decimal-এর বেশি ফল হলে rounding stage/mode এবং multiple booking components-এ verified allocation। Per-passenger Fixed unit ও count aggregation §5.6-এ settled।
- একই audience/scope-এ duplicate/conflicting rules, multicity/mixed-airline fare-এর route/carrier matching এবং reverse-route matching। আগের latest-update/earliest-leg tie-breaker স্বয়ংক্রিয়ভাবে গ্রহণ করা হবে না।
- অতিরিক্ত internal-only fields-এর exposure policy; public breakdown ও signed `discountPrice` §5.6 অনুযায়ী হবে। পুরোনো Gross suppression প্রয়োগ হবে না।
- Rule amounts, validation bounds ও rule lifecycle details। Management authority ও trusted client-to-audience/agent assignment §3-এ final।

### 5.6 Final Markup Response Shape & Discount Calculation — 2026-09-07

এই section ব্যবহারকারীর final passenger-level pricing ও response projection contract। §5.5-এর audience ও একটিমাত্র winning rule selection অপরিবর্তিত থাকবে। Supplier response contract/shape অপরিবর্তিত থাকবে; commercial `totalPrice` হবে আমাদের Selling Fare।

#### Strict response preservation contract

- Markup processing কোনো response field add, remove, rename বা restructure করবে না। Original field names/casing, nesting, object/array structure, JSON value types এবং missing বনাম null distinction অপরিবর্তিত থাকবে। Unknown supplier fields-ও preserve করতে হবে; fixed DTO serialization দিয়ে drop করা যাবে না।
- শুধু pricing rules-এ নির্ধারিত existing numeric values পরিবর্তন করা যাবে: passenger-level `totalPrice` ও `discountPrice`, top-level `totalPrice`, এবং `bookingComponents[]`-এর `totalPrice` ও `discountPrice`। কোনো field absent হলে নতুন করে যোগ হবে না; required pricing value missing/invalid হলে validation/mapping error হবে, invented field/value নয়।
- `basePrice`, `taxes`, `ait`, `equivalentBasePrice`, `serviceCharge`, `totalExtraServicePrice`, `agentAdditionalPrice` এবং অন্য সব fields markup transformation-এ অপরিবর্তিত থাকবে। Top-level `discountPrice` বা অন্য pricing path পরিবর্তনের জন্য explicit pricing rule প্রয়োজন; নামের মিল দেখে recursive replacement নয়।
- Supplier-এর original individual passenger `totalPrice`-ই calculation basis; reconstructed Base+Tax, booking component aggregate, segment fare বা আগে markup করা value basis নয়।
- Markup step offer/reference/metadata values পরিবর্তন করবে না, array entries reorder/add/remove করবে না। পূর্বনির্ধারিত aggregation/lowest-offer selection ও platform reference binding পৃথক দায়িত্ব; এগুলো markup-এর field-edit permission নয় এবং public contract-এর field structure বদলাতে পারবে না।
- OpenAPI ও contract verification-এ before/after recursive shape equality এবং allowed pricing paths ছাড়া সব values-এর equality যাচাই করতে হবে; extra/unknown fields, nulls এবং nested arrays-সহ fixtures থাকবে।

#### Passenger-level calculation

প্রতিটি supported passenger type t-এর জন্য supplier original individual `totalPrice` S_t, count q_t এবং applicable winning rule ব্যবহার করতে হবে:

```text
Percentage: M_t = S_t × percentage / 100
Fixed:      M_t = configured fixed amount
New passenger totalPrice U_t = S_t + M_t
Gross G_t = supplier basePrice + supplier taxes
Selling Before AIT = U_t − supplier ait
New discountPrice D_t = G_t − (U_t − supplier ait)
Final top-level totalPrice = sum(U_t × q_t)
```

- Fixed amount প্রতিটি individual passenger fare-এ একবার apply হবে, তারপর count multiply হবে। Adult-reference proportion বা per-segment markup নয়।
- Supplier original `totalPrice`-এ AIT ইতিমধ্যে included; markup basis-এ AIT থাকবে কিন্তু total-এর সঙ্গে আবার AIT যোগ হবে না।
- `basePrice`, `taxes`, `ait`, `equivalentBasePrice` এবং উদাহরণের `serviceCharge` supplier original breakdown হিসেবে অপরিবর্তিত থাকবে। Markup বসাতে base/tax বাড়ানো যাবে না।
- Gross শুধু Base + Taxes; AIT Gross-এর অংশ নয়। Discount comparison-এর জন্য new total থেকে original AIT বাদ হবে।
- `discountPrice` negative, zero বা positive হতে পারে। Negative মানে Selling Before AIT Gross-এর বেশি। Negative discount clamp/hide করা বা Gross suppress করা যাবে না।
- Supplier-এর পুরোনো `discountPrice` overwrite করে formula অনুযায়ী নতুন মান হবে। Original discount nonzero থাকলে new discount সবসময় minus markup হবে এমন অনুমান করা যাবে না।
- `ins: null`-এর মতো absent passenger fare shape preserve হবে; count zero হলে total-এ contribution zero। Positive count-এর fare missing/null হলে validation/mapping error, invented zero fare নয়।
- Supplier precision ও original values preserve করতে হবে। User-approved §5.7 অনুযায়ী individual selling price ২ decimal half-up round করে তারপর count aggregate হবে; supplier original numeric values ও final examples-এর fractional values preserve হবে।

#### Verified 3% example

| Type | Count | Supplier individual total | Markup | New individual total | Original base | Original taxes | Original AIT | New discount |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ADT | 2 | 35,040.00 | 1,051.20 | 36,091.20 | 24,666.00 | 10,284.00 | 90.00 | -1,051.20 |
| CHD | 1 | 26,855.00 | 805.65 | 27,660.65 | 18,500.00 | 8,284.00 | 71.00 | -805.65 |
| CNN | 1 | 24,855.00 | 745.65 | 25,600.65 | 18,500.00 | 6,284.00 | 71.00 | -745.65 |
| INF | 1 | 9,031.00 | 270.93 | 9,301.93 | 6,167.00 | 2,839.00 | 25.00 | -270.93 |

CHD response:

```json
{
  "discountPrice": -805.65,
  "ait": 71.0,
  "totalPrice": 27660.65,
  "basePrice": 18500.0,
  "equivalentBasePrice": 18500.0,
  "taxes": 8284.0,
  "serviceCharge": 0.0
}
```

```text
Original all-passenger total = 130,821.00
Total markup = 3,924.63
New all-passenger total = 36,091.20×2 + 27,660.65 + 25,600.65 + 9,301.93
                        = 134,745.63
Original aggregate Base = 92,499.00
Original aggregate Taxes = 37,975.00
Original aggregate AIT = 347.00
Aggregate discount = (92,499 + 37,975) − (134,745.63 − 347)
                   = -3,924.63
```

#### Booking components ও top-level response

Single-component example-এ component entire offer cover করে, তাই তার `totalPrice` final aggregate total এবং `discountPrice` component Gross − (new component total − original component AIT) হবে:

```json
{
  "totalPrice": 134745.63,
  "basePrice": 92499.0,
  "taxes": 37975.0,
  "bookingComponents": [
    {
      "discountPrice": -3924.63,
      "totalPrice": 134745.63,
      "basePrice": 92499.0,
      "totalExtraServicePrice": 0.0,
      "taxes": 37975.0,
      "ait": 347.0,
      "fareReference": "",
      "agentAdditionalPrice": 0.0
    }
  ]
}
```

- Passenger, component এবং top-level selling totals একই passenger coverage/count অনুযায়ী reconcile হবে। Component/top-level-এ markup পুনরায় apply হবে না; passenger pricing থেকে aggregate/project হবে।
- একাধিক component থাকলে প্রতিটিতে whole-offer total copy করা যাবে না। Supplier-এর verified component coverage ও allocation mapping প্রয়োজন; component মানেই route/segment নয়। Final rounding/allocation contract ছাড়া component split অনুমান করা যাবে না।
- Existing response field names, nesting ও applicable nullability preserve হবে। Top-level field না থাকলে শুধু passenger/component example দেখে নতুন `discountPrice`/`ait` field invent করা যাবে না। OpenAPI-তে mapped schemas/examples থাকতে হবে।
- Nonzero ancillary/service-charge বা supplier aggregate mismatch-এর ক্ষেত্রে supplied price semantics verify করতে হবে; passenger total-এর বাইরে fee অনুমান করে যোগ বা বাদ দেওয়া যাবে না।

#### সব trip type ও lifecycle

One-way, round-trip, multicity, direct/connecting flights, single/multiple passengers এবং ADT/CHD/CNN/INF/INS বা অন্য supported type-এ একই নিয়ম। Markup final passenger fare-এর ওপর, individual flight segment-এর ওপর নয়। Lowest selection হবে একই passenger coverage-এর original supplier top-level totalPrice দিয়ে, আমাদের markup ছাড়া। নির্বাচিত offer-এর user-facing Selling Fare passenger-level markup ও rounding থেকে count-aggregate হবে। Search/RePrice এবং Book/Issue/Confirmed price projection-এ accepted pricing version বজায় থাকবে; original supplier values private source snapshot হিসেবে সংরক্ষিত থাকবে, overwritten selling response পুনরায় markup basis হবে না।

### 5.7 Evidence-based pending decisions — implementation direction (2026-09-08)

Areas #1–#4 deferred; previously suggested defaults are not approved business rules. এগুলোর জন্য implementation অপেক্ষা করবে না; foundation, final auth, adapters, evidence capture এবং established pricing behaviours নিয়ে কাজ চলবে। সংশ্লিষ্ট ambiguous case এলে affected behaviour final/execute করার আগে decision নিতে হবে; independent work চলতে পারে।

1. **No matching markup rule — approved:** Applicable audience/scope fallback শেষেও rule না মিললে `PRICING_CONFIGURATION_ERROR` দিতে হবে। Zero-markup/Gross/supplier-price fallback হবে না; administrator-কে applicable rule configure করতে হবে।
2. **Rounding — approved:** Original supplier per-passenger `totalPrice`-এর ওপর exact markup calculation হবে; resulting individual selling price ২ decimal half-up rounding হবে। তারপর count multiply ও aggregate হবে। Discount rounded selling price থেকে derive হবে; markup amount আলাদা round হবে না, supplier Base/Tax/AIT বদলাবে না। Currency identity/connection currency contract এখনও পৃথক pending integration বিষয়।
3. **Rule matching/duplicates:** Real one-way, return, multicity, connecting ও mixed-airline response দেখে route/carrier matching স্থির হবে। Existing final audience priority অপরিবর্তিত; plating-carrier selection, first-leg tie, reverse-route behaviour বা duplicate rejection/update policy অনুমান করা যাবে না। Active duplicate policy approved in the Admin markup management update: same audience/agent/airline/route collision is rejected with 409; no automatic replacement। Route/carrier matching remains unresolved।
4. **Multiple bookingComponents:** Original passenger-level totalPrice থেকে marked-up passenger totals authoritative selling basis। Actual components-এর meaning/coverage inspect না করে proportional, equal বা অন্য arbitrary allocation করা যাবে না। Shape অপরিবর্তিত রেখে component representation evidence অনুযায়ী নির্ধারণ হবে।

প্রকৃত business decision প্রয়োজন হলে assistant report করবে:

1. Actual supplier behaviour/response, credentials/PII redacted এবং প্রয়োজনীয় field/precision/coverage preserved।
2. কেন বর্তমান final rules দিয়ে case-এর সিদ্ধান্ত হয় না এবং সিদ্ধান্ত কেন প্রয়োজন।
3. Available safe options ও তাদের প্রভাব।
4. Recommended option ও evidence-based কারণ।

এরপর behaviour lock করার আগে user approval-এর জন্য অপেক্ষা করতে হবে। No silent business-rule invention। Raw evidence নিরাপদভাবে সংরক্ষণ করতে হবে; snapshots-এ original prices overwrite নয়। Evidence সংগ্রহ supplier mutation/live booking-এর blanket authorization নয়; approved scope ও execution permissions প্রযোজ্য থাকবে।

## 6. Public API contract

Supplier-compatible paths এবং field casing বজায় রেখে OpenAPI-তে নির্দিষ্ট contract দিতে হবে:

| Operation | Method / path | Purpose |
|---|---|---|
| Machine token exchange | `POST /auth/token` — contract: `docs/AUTHENTICATION.md` | Client ID + Client Secret → opaque Bearer token, 30-minute lifetime |
| Search | `POST /api/Search` | Aggregated class/fare options |
| FareRules | `POST /api/FareRules` | Selected offer-এর fare rules |
| RePrice | `POST /api/Reprice` | Selected offer-এর live price validation |
| Book | `POST /api/Book` | Booking অথবা explicitly permitted direct issue |
| Cancel | `POST /api/Cancel` | Eligible held booking cancellation |
| NewTicket | `POST /api/ticket/NewTicket` | Held booking ticket issue |
| PNR | `POST /api/pnr` | Booking status এবং latest ticket deadline |
| Ticket details | `GET /api/B2BReport/AirTicketingDetails/{uniqueTransID}/{status}` | Client-owned ticket/report details |

- Pipeline responses-এ supplier-এর `item1` / `item2` envelope বজায় রাখতে হবে। Report endpoint-এর documented পৃথক response shape রাখতে হবে। আমাদের machine token exchange §3-এর পৃথক platform authentication contract; supplier Login adapter-এর internal operation।
- `item1.airSearchResponses[]`-এ প্রতিটি retained fare option supplier-compatible entry হবে; canonical internal shape সরাসরি public API-তে expose করা বাধ্যতামূলক নয়।
- আমাদের references সংশ্লিষ্ট public fields-এ বসবে। Supplier tokens/credentials/raw references client-এর জন্য প্রয়োজন হবে না।
- HTTP status, business failure, validation failure, expired reference, partial supplier failure ও permission errors-এর mapping স্থির করতে হবে। Success/failure field semantics সঙ্গতিপূর্ণ হতে হবে।
- Compatibility মানে specified shape compatibility; supplier-এর সঙ্গে unverified drop-in compatibility দাবি করা যাবে না। নতুন response body metadata/extensions দিয়ে supplier shape বদলানো যাবে না। প্রয়োজনীয় platform metadata existing compatible field বা documented response header-এ দিতে হবে; transport ও versioning policy document করতে হবে।

## 7. Reference ownership ও workflow

- আমাদের opaque transaction/offer/price/booking references database record-এর সঙ্গে map হবে। Client arbitrary supplier reference পাঠিয়ে operation করতে পারবে না।
- Mapping-এ owner client, source connection/account, supplier-issued references, expiry, selected fare এবং last validated price থাকবে। Supplier references অপরিবর্তিতভাবে ব্যবহার করতে হবে।
- RePrice থেকে refreshed reference এলে পরবর্তী operation-এ সেটি ব্যবহার করতে হবে।
- Search → FareRules/RePrice → Book → Cancel/PNR/NewTicket/Report একই selected supplier connection ব্যবহার করবে।
- Search/price reference expiry এবং booking/ticket retention আলাদা lifecycle হবে। Expired offer দিয়ে booking ঠেকাতে হবে।
- Booking state ও supplier confirmation durably persist করতে হবে। Latest PNR deadline authoritative হিসেবে ব্যবহার করতে হবে; timezone contract নিশ্চিত করতে হবে।
- Supplier যখন Search এবং RePrice-এ `bookable=false` দেয়, Book সরাসরি issue করতে পারে। এমন Book-এর জন্য ticketing permission, explicit direct-issue intent এবং ticketing enablement প্রয়োজন হবে। Intent কীভাবে পাঠানো হবে তা public contract-এ নির্ধারণ করতে হবে।
- Issued ticket পাওয়া গেলে আর NewTicket পাঠানো যাবে না। Issued ticket-এ hold-cancellation endpoint প্রয়োগ করা যাবে না।

## 8. Reliability ও transaction controls

- Book, Cancel এবং NewTicket-এ client-scoped idempotency contract থাকতে হবে। একই key ও payload পুনরায় এলে duplicate supplier mutation হবে না; একই key-তে ভিন্ন payload reject করতে হবে।
- Concurrent requests-এর ক্ষেত্রে database constraints/locking দিয়ে duplicate booking/issue ঠেকাতে হবে।
- Supplier mutation timeout মানেই failure নয়। Outcome unknown state সংরক্ষণ করে available PNR/report দিয়ে reconciliation করতে হবে; blind retry বা অন্য supplier দিয়ে booking নয়।
- Supplier-side idempotency নিশ্চিত না হলে exactly-once execution guarantee দাবি করা যাবে না। Automated reconciliation অসম্ভব হলে authorized manual resolution দরকার হবে।
- Read operations-এ bounded timeout/retry/backoff থাকবে। Authentication retry বা generic 5xx retry দিয়ে mutation অনিরাপদভাবে পুনরায় পাঠানো যাবে না।
- Audit trail, request correlation এবং redacted operational logs থাকবে। Credentials, tokens, passport/document content বা পূর্ণ passenger payload log করা যাবে না।
- User-approved hold policy: hold booking-এ payment/balance/credit check লাগবে না; valid client booking permission এবং supplier enablement যথেষ্ট, অন্যান্য quote/passenger checks বহাল। Instant purchase/ticketing-এর payment/authorization policy পৃথক; API permission নিজে payment-এর প্রমাণ নয়।

## 9. PostgreSQL data requirements

Conceptual records; implementation-এ physical schema নির্ধারিত হবে:

- B2B API clients/Client IDs, secret verification hashes, opaque token verification records/expiry/revocation, permissions ও client status।
- পৃথক human Admin/Super Admin identities, secure authentication/session records, administrative roles, bootstrap completion ও immutable security audit।
- Supplier connection metadata/capabilities; supplier sessions persisted হলে encrypted sensitive values।
- Search sessions, canonical offers, source-offer mappings ও expiry।
- RePrice snapshots, accepted totals ও reference versions।
- Markup rules: B2C/All B2B/Specific Agent audience, optional agent linkage, airline/route scopes, Fixed/Percentage value এবং rule versions; pricing snapshots-এ winning rule ও resolved audience।
- Bookings, passengers/contact details, supplier references ও lifecycle transitions।
- Tickets, supplier reconciliation status ও ticket reports।
- Idempotency records, mutation attempts এবং audit events।

Foreign keys, ownership constraints, monetary precision, indexed reference lookups এবং retention/cleanup policy থাকতে হবে। Passenger data access সীমিত থাকবে; sensitive persisted data protection ও backup/restore ব্যবস্থা deployment design-এ নির্ধারণ করতে হবে।

## 10. Swagger / OpenAPI

- Swagger UI এবং downloadable OpenAPI JSON দিতে হবে।
- প্রত্যেক endpoint-এর purpose, auth, required/optional fields, exact casing, enums, response schemas এবং success/error examples থাকবে।
- One-way, round-trip, multicity, multiple fare classes, price change, direct issue ও partial search result-এর examples থাকবে। Examples-এ real credentials/passenger data থাকবে না।
- Swagger Authorize দিয়ে আমাদের Bearer token ব্যবহার করে Try it out করা যাবে।
- Documentation UI-তে active environment পরিষ্কার দেখাতে হবে। Book/NewTicket/direct-ticketing যে বাস্তব transaction করতে পারে তা operation description-এ স্পষ্ট থাকবে; backend permissions ও execution gates সবসময় কার্যকর থাকবে।
- Request validation ও actual serialized responses যেন documented schema-এর সঙ্গে মেলে, contract checks দিয়ে যাচাই করতে হবে। Markup before/after shape একই হবে; শুধুমাত্র §5.6-এর অনুমোদিত existing pricing values বদলাবে।

## 11. Acceptance criteria

- Authorized admin client তৈরি করতে পারে; Client ID + Client Secret exchange-এ 30-minute opaque token পাওয়া যায়। Expired token machine exchange দিয়ে replace হয়, human login/refresh token লাগে না। Client disable/credential reset/revocation-এ existing authorization reject হয়। Cross-client resource access reject হয়।
- Agent/API client markup mutate করতে পারে না; separate authenticated authorized Admin/Super Admin পারে। Secret issue/reset ছাড়া প্রকাশ হয় না; controlled bootstrap একবারই কার্যকর এবং default admin password/public registration নেই।
- Admin session দিয়ে commercial Flight API এবং B2B token দিয়ে admin API access reject হবে। Markup create/edit/activate/deactivate সব operation-এ admin authority enforce হবে। B2B/B2C/Specific Agent context client নিজে override করতে পারবে না; bootstrap-এর পরে administrator management authorized admin access ছাড়া reject হবে।
- Mock তিন supplier-এর একই itinerary-তে Saver, Flex ও Business থাকলে তিন option-ই থাকে এবং প্রত্যেকটির সঠিক minimum total নির্বাচিত হয়।
- তিনটি single-supplier selection, তিনটি two-supplier combination এবং all-three selection-এ শুধু enabled connections call হয়; fare selection সেই subset অনুযায়ী সঠিক হয়। All-disabled অবস্থায় `NO_ACTIVE_SUPPLIERS` আসে।
- Admin supplier toggle restart ছাড়া পরের Search-এ কার্যকর হয় এবং persisted থাকে। Disabled supplier partial failure হিসেবে দেখায় না; stale unbooked offers reject হয় এবং existing bookings নিজস্ব connection/permissions দিয়ে service করা যায়।
- Different RBD, baggage, refundability বা uncertain brand match ভুলভাবে merge হয় না। Single-supplier option হারায় না।
- Passenger totals, mandatory fees, currency precision ও deterministic ties সঠিকভাবে কাজ করে।
- Markup transformation-এ recursive response shape অপরিবর্তিত এবং allowlisted pricing paths ছাড়া সব values identical থাকবে। Unknown fields ও null/absent distinction preserve হবে; নতুন metadata বা pricing field যোগ হবে না।
- Markup checks: S=30,000 + Fixed 500 → Selling Fare 30,500; S=30,000 at 2% → Selling Fare 30,600। §5.5-এর ABC example-এ চারটি expected markup 100/200/400/500 হতে হবে। No-stacking example-এ 200 হবে, 1,400 নয়।
- Agent All/All rule B2B specific rule-এর আগে জিতবে; unrelated Agent rule B2B fallback বন্ধ করবে না। B2C শুধু নিজের chain ব্যবহার করবে; arbitrary agent identity দিয়ে special pricing পাওয়া যাবে না।
- Lowest selection original Supplier total price দিয়ে হবে, markup-পরবর্তী Selling Fare দিয়ে নয়। Test-এ supplier-total winner ও selling-total winner ভিন্ন হলে supplier-total winner-ই নির্বাচিত হবে; নির্বাচিত offer-এর markup projection যাচাই হবে। RePrice acceptance/versioning এবং Book/Issue-তে markup পুনরায় যোগ না হওয়ার checks থাকবে। §5.5-এর unresolved edge-case decisions অনুযায়ী additional checks যুক্ত হবে।
- §5.6 fixture-এ CHD total=27,660.65 ও discount=-805.65; counts 2/1/1/1-এ top-level এবং single-component total=134,745.63, component discount=-3,924.63। Base/taxes/AIT অপরিবর্তিত, AIT double-count নয়; null INS preserve হবে।
- Fixed 500 এবং তিনটি individual passenger হলে aggregate markup 1,500 হবে, flight segments যতই থাকুক। Existing supplier discount nonzero থাকলেও formula দিয়ে recompute হবে; negative discount suppress হবে না।
- One supplier failure-এ valid partial result পাওয়া যায়; all-failed outcome স্পষ্ট থাকে।
- Selected offer-এর পরের সব operation সঠিক owner ও supplier connection/reference দিয়ে চলে।
- Changed fare acceptance, expired references, duplicate requests এবং ambiguous mutation timeout যথাযথভাবে handle হয়।
- Direct issue ticket permission ছাড়া হয় না; issue হয়ে গেলে দ্বিতীয়বার NewTicket যায় না।
- Database migrations, authentication/isolation, aggregation, state transitions ও API schemas-এর relevant tests pass করে।
- Swagger-এ schemas/examples দেখা এবং authorized requests test করা যায়। Supplier live/UAT validation আলাদা অনুমোদিত ধাপ; mock pass-কে live credential validation বলা যাবে না।

## 12. Implementation sequence

- [x] Step 1. Rust/Axum foundation, configuration, migrations এবং initial OpenAPI setup; #1–#4-এর speculative decisions দিয়ে কাজ block নয়।
- [x] Step 2. Final §3 অনুযায়ী machine credentials/token exchange, পৃথক human admin authentication/bootstrap, authorization ও ownership isolation।
- [ ] Step 3. Shared supplier adapter, per-connection authentication, verified/redacted response fixtures ও evidence capture। §5.7 অনুযায়ী relevant cases-এ decision report/approval।
- [ ] Step 4. Canonical normalization, final audience-based markup/fallback engine, per-option lowest original Supplier total price selection এবং Search/FareRules।
- [x] Step 5. Public RePrice, versioned reference persistence ও explicit local price acceptance (2026-09-08); supplier coverage limitations remain documented।
- [ ] Step 6. Book/Cancel/PNR, idempotency ও reconciliation।
- [ ] Step 7. NewTicket, direct issue, reports এবং commercial execution controls।
- [ ] Step 8. Contract/integration verification, Swagger completion এবং separately authorized supplier-environment validation।

প্রতিটি ধাপের সঙ্গে সংশ্লিষ্ট tests ও Swagger documentation update হবে।

### Implementation verification log

- **Step 1 — complete, 2026-09-08:** Git/Rust/Axum foundation, validated configuration, PostgreSQL pool, explicit versioned migration command, migration-checksum readiness, liveness, generated request IDs, redacted JSON logs, graceful shutdown, vendored Swagger UI ও OpenAPI। Initial supplier configuration ও append-only audit schema তৈরি। `.env` Git-ignored এবং অক্ষত।
- **Step 1 validation:** `cargo check`, `cargo clippy --locked --all-targets -- -D warnings`, three HTTP/configuration tests, এবং local PostgreSQL 18.3-এ migration repeatability/constraints/audit immutability/checksum mismatch test পাস। Actual `migrate` command dedicated local database-এ সফল। Supplier network validation করা হয়নি।
- **Step 2 — complete, 2026-09-08:** `POST /auth/token`, 1,800-second opaque token, Argon2id secret hashes, credential reset/rotation/revocation, disable invalidation, পৃথক admin sessions/login/logout, concurrency-safe one-time bootstrap, protected client/admin management, trusted pricing context, permissions, shared rate limits ও ownership helper। `docs/AUTHENTICATION.md`-এ wire/security contract আছে। Full flight-resource integration ও markup-management endpoints সংশ্লিষ্ট পরবর্তী ধাপে বাকি।
- **Step 2 validation:** Real local PostgreSQL integration test-এ concurrent bootstrap, forbidden second bootstrap, token lifetime/expiry/renewal, reset/revoke/disable/re-enable invalidation, admin/machine separation, permissions, ownership isolation, Super Admin restrictions, admin disable/logout ও authentication/client rate limits পাস।
- **Step 3 — partial:** Shared read-only supplier transport, separate hosts/caches, expiry/401 refresh coordination, bounded retry/timeout/response size, redirect rejection; protected persisted supplier controls, optimistic versioning, active snapshot ও stale-unbooked epoch checks তৈরি। Local HTTP mocks ও PostgreSQL checks পাস। Complete verified/redacted supplier fixtures এবং approved supplier-environment validation বাকি; live validity দাবি করা হচ্ছে না।
- **Step 4 — partial:** Verified input context-এর জন্য final audience/scope priority/fallback, no stacking, arbitrary-precision passenger calculation, signed discount ও aggregate core তৈরি; ABC example ও §5.6-এর 134,745.63 total tests পাস। No-match/duplicate signals internal pending-decision markers, final public policy নয়। Canonical supplier mapping, markup admin conflict policy, shape-preserving projection ও public Search/FareRules integration বাকি।
- **Step 5 endpoint implemented; Steps 6–8 pending:** RePrice versions/local acceptance complete within documented coverage. Booking/ticket workflows, booking idempotency/reconciliation, remaining commercial policy ও full supplier contract coverage সম্পন্ন হয়নি।
- **Latest verification:** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, 7 unit/HTTP tests এবং পৃথক PostgreSQL integration scenario suite পাস। Dedicated local database-এ তিন migration apply হয়েছে; running HTTP smoke-এ `/health/live`, `/health/ready`, `/openapi.json`, `/docs/` সব 200; OpenAPI-তে 14 paths। CI workflow লেখা হয়েছে, remote CI run করা হয়নি।
- **Evidence boundary:** `docs/evidence/SUPPLIER_READINESS.md`-এ supplied-document gaps, completed mock work এবং remaining evidence/decisions নথিভুক্ত। Original `.env` assistant পরিবর্তন করেনি। এই baseline check-এর সময় supplier calls করা হয়নি; পরবর্তী অনুমোদিত production validation নিচে নথিভুক্ত।

### Production validation update — 2026-09-08

User updated `.env` to production and explicitly authorized continued read-only validation, prohibiting issue. This authorization covers Login/Search/FareRules/RePrice for this implementation task; it is not approval for Book/Cancel/NewTicket/direct issue.

- [x] Firsttrip, Takeoff ও Triplover production Login + one-way Search + selected FareRules/RePrice verified; offer counts 27/24/27।
- [x] Triplover return ও multicity selected flow verified (7/467 offers); 2 adults + child + infant coverage।
- [x] Private original evidence captured; representative sanitized fixtures exported with numeric precision/nulls preserved।
- [x] Actual Search `item2` array বনাম FareRules/RePrice object shape recorded; Search currency absent এবং sampled RePrice BDT recorded।
- [x] Single-component Fixed markup projection এবং recursive shape/value-preservation tests added; 4 passengers × 500 = 2,000 on return/multicity verified।
- [x] Rounding/no-match policies subsequently approved and implemented: passenger-level ২ decimal half-up then aggregate; missing applicable rule → `PRICING_CONFIGURATION_ERROR`। Route/carrier/duplicate policies and multi-component evidence remain open।
- [ ] Step 3/4 overall completion remains partial: unrestricted return hit 8 MiB bound; public aggregation, currency contract, equivalence, markup management ও Search/FareRules/RePrice integration unfinished।

Production-step verification: formatting ও strict clippy passed; 11 unit/HTTP/fixture tests passed। Existing database suite remains opt-in and was not rerun for this DB-independent change.

Evidence, options, recommendations ও remaining work: `docs/evidence/PRODUCTION_VALIDATION_2026-09-08.md`। No supplier Book/Cancel/NewTicket/direct-issue call performed; existing client/admin/supplier activation data unchanged।

## 13. Open decisions ও supplied material

Available: `.env` ও `Triploaver_API_Documentation.md`। User-confirmed production credentials দিয়ে উপরের sampled read-only flows যাচাই হয়েছে। Documentation-এর UAT hosts current approved production configuration নয়; production observations ও সীমাবদ্ধতা verification log-এ আছে।

Implementation চলাকালে সংশ্লিষ্ট evidence/design ধাপে নির্ধারণ/নিশ্চিত করতে হবে; #1–#4-এর জন্য §5.7 প্রযোজ্য:

- §3-এর final auth অনুযায়ী token exchange route/wire schema, human admin session implementation ও controlled bootstrap mechanics document করতে হবে। Client ID/Secret, opaque token, 30 minutes, automatic machine renewal, revocation ও admin-only markup management settled।
- Supplier account permissions, rate limits, reference TTL, timeout reconciliation এবং mutation idempotency support।
- Canonical brand/equivalence mappings, verified supplier price/component semantics, ancillary totals। Equal-price supplier priority approved on 2026-09-09: Takeoff → Firsttrip → Triplover। Final audience/priority/fallback ও calculation §5.5-এ settled; Passenger basis, Fixed unit, count aggregation, AIT ও signed discount projection §5.6-এ settled; no-match outcome, rounding, multiple-component representation ও matching/duplicate ambiguities evidenceসহ §5.7 অনুযায়ী resolve করতে হবে। Admin-only rule management final।
- Direct-issue intent field/header, changed-price acceptance contract এবং partial-result metadata transport, supplier response body shape অপরিবর্তিত রেখে।
- Ticket deadline timezone, conditional booking fields ও undocumented `fareType` enum।
- Client credit/payment policy, supplier balance failure handling এবং operational resolution workflow।
- Deployment environment, secret management, retention ও backup policy।

Supplier document-এ referenced `MARKUP.md`, original PDF, Postman collection ও fixture package বর্তমান project-এ নেই। প্রয়োজনীয় schema ambiguities resolve করতে এগুলো বা supplier clarification লাগতে পারে। পুরোনো project-এর pricing policy অনুমান করে প্রয়োগ করা যাবে না।

## 14. Initial scope-এর বাইরে

- Rust backend-এর পরিবর্তে পুরোনো Node.js backend proxy করা।
- আলাদা supplier-এর flight legs জুড়ে নতুন itinerary তৈরি করা।
- নিজস্ব frontend বা full administrative dashboard নির্মাণ; protected admin API scope-এ আছে। পরবর্তী user approval (2026-09-10) অনুযায়ী শুধু booking reconciliation-এর coding-free Admin screen এখন scope-এ যুক্ত; full dashboard নয়।
- Payment gateway integration, client wallet/accounting এবং outbound webhooks—আলাদা requirements ছাড়া। Ticket execution-এর authorization policy অবশ্য scope-এ আছে।
- Refund, reissue ও VOID APIs/workflows এই project-এর বর্তমান scope-এ নেই; purchase flow Issue/Confirmed পর্যন্ত। Eligible unissued held booking-এর বিদ্যমান Cancel endpoint post-ticket refund/VOID নয়।


### Approved pricing implementation update — 2026-09-08

User approved the two recommendations with “প্রসিড করো” after the supplier-evidence explanation.

- [x] Exact markup → individual selling price half-up to 2 decimal places → count aggregation.
- [x] Passenger/component discount derived from rounded selling amounts; no independent markup rounding or second AIT addition.
- [x] Rule-resolution + projection pipeline fails with `PRICING_CONFIGURATION_ERROR` if no applicable rule exists. No implicit zero markup.
- [x] Tests cover below/at/above half-cent, carry to next unit, per-passenger vs final-only aggregation, original snapshot/response shape preservation, and real 4033.05 × 1.03 → 4154.04 fixture.
- [ ] Public Search endpoint, currency identity, canonical matching, markup admin rule management and other unfinished workflow steps remain open; this update does not mark those complete.

Validation: `cargo fmt --check`, strict all-target clippy and local test suite. No supplier network call, booking or ticket issue was needed for this change.


### Admin markup management update — 2026-09-08

User explicitly approved: “একই scope-এ দ্বিতীয় active rule reject করুন”. Same audience/agent + airline + route may have only one active rule. A second activation or an active edit colliding with that scope returns HTTP 409; it does not silently replace the existing rule. Inactive drafts may coexist. Edit the existing ID or deactivate it before activating another. PostgreSQL NULLS NOT DISTINCT partial unique index enforces All/All and other nullable scopes under concurrency. Currency is not a separate duplicate scope.

- [x] Protected create/list/get/edit/activate/deactivate admin markup API; five operations documented with working examples under Swagger's `Markup rules` tag.
- [x] PostgreSQL persistence, exact decimal amount storage, optimistic version checks, immutable rule-version snapshots and transactional audit.
- [x] Tests: machine-token rejection, input validation, concurrent edits, concurrent same-scope activation, conflicting active edit rollback, deactivate then activate replacement, exact amount storage, version/audit integrity.
- [ ] Step 4 overall remains partial: canonical mapping, supplier/account currency contract, return/multicity/mixed-airline scope matching and public Search/FareRules integration are not complete. Multi-component projection evidence remains open.

Usage: `docs/MARKUP_API.md`. The API does not call suppliers or issue tickets. Admin/client identities and existing supplier activation settings are preserved.

Markup management validation completed: 13 local unit/HTTP/fixture tests, full disposable PostgreSQL migration/authentication/markup integration suite, formatting and strict clippy passed. Five migrations applied to the working local database; readiness 200 and five new authenticated Swagger operations verified. No test rules were created in the working database; public Search remains unfinished.


### Initial public Search/FareRules update — 2026-09-08

User explicitly confirmed all three configured Search accounts use BDT. Added FIRSTTRIP_CURRENCY/TAKEOFF_CURRENCY/TRIPLOVER_CURRENCY=BDT; other `.env` values preserved. Per-offer currency remains absent in the supplier response; item1.currency is null in the captured samples. Currency comes from the trusted configuration, exposed in a response header.

- [x] Initial machine-authenticated `POST /api/Search`: active-connection snapshot, parallel bounded calls, partial/all-failure handling, database active-rule lookup, approved markup/rounding and private original/pricing/reference persistence.
- [x] `POST /api/FareRules`: owner/expiry/segment validation, original supplier reference routing, platform reference rebinding; no mutations.
- [x] Local tests cover all 7 supplier subsets, no-active/no-rule, partial/all failure, admin-token rejection, price application, unknown field preservation and foreign/tampered/expired references.
- [x] Isolated production read-only public-flow smoke: 78 Search offers, no failed active connections, Fixed 500 verified, FareRules 200. Temporary identities/rules were confined to a separate local test database.
- [ ] Step 4 remains partial: canonical equivalence/lowest-supplier deduplication unfinished; uncertain options remain separate. Only unambiguous single-route direct offers are priced initially; complex matching, branded fares and multi-component projection remain gated.
- [ ] Existing supplier summary/filter metadata is retained from the first successful source, not recalculated as aggregate selling summaries; `X-Search-Summary-Scope` documents the boundary. Full aggregate summaries and final response/error schema work remain open.
- [x] Public RePrice now implemented with current scope/coverage guards; all booking/ticket operations remain unimplemented. Earlier unresolved scope decisions are not expanded by this endpoint.

Run and test contract: `docs/SEARCH_API.md`. User's working client/rule/supplier activation settings are preserved. No Book/Cancel/NewTicket/direct issue occurred.

### Swagger supplier routing fix — 2026-09-08

- [x] Fixed duplicate OpenAPI operation IDs (`list` and `update`) shared by supplier and markup endpoints, which caused Swagger supplier requests to target markup rules. Supplier operations now use unique `list_suppliers` / `update_supplier` IDs under the Suppliers tag.
- [x] Added regression validation that every generated operation ID is globally unique; all 3 foundation tests passed. Rebuilt/restarted the local server and verified served OpenAPI uniqueness and ready status. Existing supplier settings and markup rules preserved. Reload Swagger to discard old operation state.

### Approved complex All/All markup exception — 2026-09-08

User approved proceeding with complex/codeshare markup when all active candidate rules for the client's audience/agent and configured currency have no airline or route restriction. This supersedes the earlier pending All/All decision; it does not approve general complex scoped matching.

- [x] Search accepts complex context with exclusively All/All candidates, preserving existing agent-first/B2B fallback and per-passenger pricing.
- [x] Any airline- or route-specific candidate keeps ambiguous offers gated by COMPLEX_SCOPE_MATCHING_UNRESOLVED. No first-leg inference. No-match, branded-fare and component-coverage guards remain.
- [x] Regression test uses the captured multicity fare: four passengers with fixed 500 add exactly 2000; agent priority and airline/route-specific rejection checked. All 16 default tests and all-target Clippy passed. Rebuilt/restarted local server; readiness verified. No new production supplier request was made for this change.
- [ ] General complex scoped matching and the other previously documented incomplete Search features remain pending.

### User-requested 5-passenger Search audit — 2026-09-08

- [x] Executed the exact supplied one-way, round-trip and multicity requests against each of the three production suppliers through the real public Search router, with isolated local test configuration and fixed BDT 500 All/All markup. Nine supplier/request combinations completed; no production mutations.
- [x] Independently checked all 1,177 offers returned by five successful combinations: per-passenger HALF_UP totals, signed discounts, five-passenger aggregate +2500, component totals/discounts, unchanged structures and nonprice/nonreference data. See `docs/evidence/REQUESTED_SEARCH_MARKUP_AUDIT_2026-09-08.md`.
- [ ] Firsttrip and Triplover round-trip: adapter TooLarge at existing 8 MiB cap, public 503. Full bodies/pricing not verified.
- [ ] Triplover one-way/multicity: 20 of 251 / 34 of 272 original offers have component tax versus passenger aggregate tax discrepancies. Current validation rejects the entire response with 422 SUPPLIER_PRICING_COVERAGE_UNSUPPORTED. Supplier component semantics remain unresolved; do not claim all Search cases pass.

Review completed, not full Search acceptance. Working client/supplier/markup settings remain unchanged. Diagnostic test completion does not imply failed API combinations passed.


### RePrice endpoint implementation — 2026-09-08

- [x] Public `POST /api/Reprice`: client-owned Search references, same supplier/availability epoch, expiry, passenger, itinerary, currency and existing pricing coverage guards.
- [x] Current winning markup rule/version applied once to refreshed original fares; original/selling/reference snapshots saved per successful pricing revision.
- [x] `POST /api/Reprice/accept`: explicit local acceptance bound to the latest unexpired priceCodeRef; idempotent retry, superseded/foreign/context-changed rejection. No supplier mutation.
- [x] Migration 0007 and OpenAPI routes/schemas; integration documentation in `docs/REPRICE_API.md`.
- Earlier entries marking public RePrice unimplemented describe the earlier milestone. Step 5 endpoint/version/acceptance implementation is now present; Book integration and supplier-specific unsupported tax/component cases remain separate work.
- Existing strict pricing guard remains unchanged in policy: no CNN tax rewriting, no automatic per-offer exclusion, no direct purchase/hold tests introduced.

- Validation: all ordinary tests and Clippy passed; disposable PostgreSQL integration tests passed. Public production read-only smoke returned 78 Search offers (not partial), FareRules 200, RePrice 200 and local acceptance 200; original/selling snapshots and one-time fixed BDT 500 markup verified. No supplier booking/ticket mutation occurred. Main running database/server deployment was not changed.


### Booking hold implementation — 2026-09-08

- [x] Mock-tested `POST /api/Book`: accepted latest quote, passenger validation, same-supplier refreshed references, client ownership and enablement checks.
- [x] Migration 0008: durable dispatch reservation, client/key request hash, one reservation per offer, held/unknown response and PNR/deadline persistence, redacted outcome audit. No blind supplier mutation retries.
- [x] Owner-only local status and read-only PNR reconciliation when known references are available; otherwise manual reconciliation required. Unresolved reservations cannot be automatically cleared/retried.
- [x] Accepted selling fare is projected from the stored snapshot; no second markup calculation. RePrice/acceptance blocked after booking reservation.
- [ ] Production booking readiness: approved payment/credit authorizer remains unimplemented; production denies commercial authorization by default. No live Book/Cancel/NewTicket called, no direct issue, no live environment/settings enabled.
- [ ] Direct-issue/payment policy, manual reconciliation resolution, ticket lifecycle and production PII protection/retention remain future work. Step 6 is not claimed fully production-complete.
- Contract: `docs/BOOKING_API.md`.


### Intentional repeat booking — user-approved 2026-09-08

- A user may intentionally book the same offer/itinerary/passengers multiple times. The platform must not reject a request solely because that offer was previously booked. Supplier restrictions still apply.
- New client-scoped `Idempotency-Key` means a separate user booking intent; same key plus same normalized payload is a retry and must never resend supplier Book. Same key plus changed payload remains a conflict.
- System retries must retain their original key; they must not automatically manufacture new intent keys. A separately initiated user request is allowed even if another intent remains pending/unknown, without changing that original intent's reconciliation state.
- Migration 0009 removes the unique-offer reservation restriction while preserving client/key uniqueness and all ownership/quote/commercial checks. Earlier Step 6 notes about one booking per offer and post-book RePrice blocking are superseded.
- RePrice/acceptance remain available for new intents; each booking keeps its own accepted price snapshot. No live booking or settings changes occurred for this update.


### Hold payment policy — user-approved 2026-09-08

- Hold booking requires no payment, wallet balance or credit authorization. The earlier production `booking_authorized` deny-all hook is removed.
- Client `booking` permission, supplier Search/booking enablement, environment booking flag and valid accepted RePrice/passenger/reference checks remain required.
- Both Search and RePrice must show `bookable=true`. Instant purchase has no hold; direct issue remains unsupported and is not enabled by this policy.
- No live Book, environment update, main database migration or running-server restart was performed for this policy change. Earlier commercial-authorizer-pending notes are superseded for holds only.


### Original supplier-total selection — user-approved 2026-09-09

- [x] Supplier selection uses original totalPrice for the full passenger mix before markup, with exact decimal comparison. Equal totals prefer Takeoff → Firsttrip → Triplover. Markup and selling-price rounding do not change supplier ranking.
- [x] Search selects within its active supplier snapshot and configured currency, then applies the existing winning markup rule to each selected original offer. Only selected offers are persisted with their original supplier references for FareRules/RePrice/Book.
- [x] Conservative equivalence compares complete reported itinerary/fare attributes and all remaining unknown fields exactly after removing evidenced transport references and quoted total/discount fields. Distinct RBD/service class/fare basis, baggage, refundability, passenger counts and route/segment order remain separate. Optional cabin-label handling follows the bookingClass update below. Base/tax/AIT and fee metadata remain part of the conservative key.
- [x] Missing required attributes other than optional cabinClass, codeshare offers and ambiguous route alternatives remain separate. No guessed cabin, brand aliases, baggage-unit conversion or component allocation is introduced. Existing branded-fare and pricing-coverage errors remain in place for all source offers, including potential losers.
- [ ] Step 4 is still partial: broader canonical equivalence, differing price-breakdown/display metadata normalization, general complex scoped markup, branded/multiple-component fares and aggregate Search summaries remain open. This update does not claim every real cross-supplier duplicate is merged.
- Validation: local unit/fixture and disposable PostgreSQL integration coverage includes all seven active subsets, per-class winners, source ownership/routing, tie priority, exact sub-cent comparison, uncertain attributes and a lower original total whose rounded selling total is higher. No supplier network call or deployment is part of this update.


### bookingClass-based comparison — user-approved 2026-09-09

- [x] `bookingClass` (Q/V/etc.) is the required class/RBD identity alongside `serviceClass` and the other existing itinerary/fare checks. Null, missing or empty `cabinClass` does not prevent matching and is not inferred from RBD or Search cabin input.
- [x] Optional cabin labels are excluded from the main comparison key. If explicitly reported nonempty cabin labels conflict for the same segment within an otherwise matching group, all offers in that group remain separate; a missing label cannot bridge conflicting cabins. This check is independent of supplier response order.
- [x] Original supplier responses retain their cabin values and null/missing shape. Q and V remain distinct options; supplier original total and Takeoff → Firsttrip → Triplover tie priority remain unchanged.
- Validation uses captured Triplover Q/null-cabin offers in all seven active-supplier subsets, plus unit regressions for null/missing/known cabin matching, conflicting labels in every ordering, distinct RBD and source-shape preservation. No supplier network call or deployment is part of this update.


### Production selection validation — 2026-09-09

- [x] Current public router verified against all three production Search accounts: 84 raw offers → 45 retained offers, 39 duplicates removed, no partial supplier failure. An independent Decimal audit verified every retained supplier-total winner and fixed-500 selling projection.
- [x] Representative selected Firsttrip/Takeoff/Triplover public FareRules → RePrice → local acceptance flows passed. No Book/Cancel/Issue was called.
- [ ] Full servicing acceptance remains open: sampled Triplover VQ FareRules returned supplier business failure; a separate Takeoff BG RePrice sample returned public 502 without a captured root cause. Final passing representative samples do not erase those observations.
- Evidence and scope: `docs/evidence/SUPPLIER_SELECTION_VALIDATION_2026-09-09.md`. These are isolated local-router tests against real suppliers, not authenticated tests against the deployed API.


### RePrice selected directions — 2026-09-09

- [x] RePrice accepts the ordered segment refs of exactly one complete Search direction per route and resolves them to the saved supplier refs. Partial, foreign, reordered, extra and ambiguous selections are rejected before supplier calls.
- [x] Returned flights must match the chosen ordered segment endpoints, airline/flight number and departure; one returned direction per route is required.
- [x] Migration 0010 stores selection indices, public/supplier segment refs and chosen original directions on each new successful pricing revision. Historical revisions remain null; original Search snapshots are unchanged.
- [x] Unit and disposable PostgreSQL tests cover selection, original-ref forwarding, persistence, wrong-flight rejection and continued null-ref rejection. Existing pricing/ownership/expiry/acceptance checks remain in place.
- [ ] Supplier null RePrice segment references and aggregate tax mismatches still need contract resolution. No tax rewrite or Search-reference fallback is introduced.
- Contract: [RePrice API](docs/REPRICE_API.md). This local implementation has not been deployed.
- Live validation: 13 Search calls and 12 public RePrice samples across the full matrix and focused MH check. Selected MH forwarded 6 of 14 available refs and got supplier success; public 422 remains for six null refreshed refs. Full matrix: 6/10 RePrice passed; detailed evidence in [selected-direction verification](docs/evidence/SELECTED_DIRECTION_REPRICE_2026-09-09.md). No live booking or deployment.

### Supplier-reported fixes rechecked — 2026-09-09

- User requested fresh production read-only verification after suppliers reported all issues resolved. Current local public router used isolated databases and unchanged validation; no deployment or booking/ticketing calls.
- [x] All 12 Search matrix scenarios now return 200 without partial supplier failures; focused Takeoff MH Search also passes. Independent Decimal audit found no pricing discrepancies in 6,735 raw offers and verified markup on 6,723 retained selling offers. Earlier aggregate-tax failures were not reproduced in this run; historical failure reports remain evidence of earlier behavior.
- [x] All 22 successful RePrices passed independent markup checks. All 38 sampled requests forwarded the selected original supplier references correctly. Firsttrip CA oneway passed both fresh samples.
- [ ] RePrice remains incomplete: 22/38 samples passed; 11 returned 422 because successful supplier responses contained null segment refs (Firsttrip, Takeoff and Triplover), and 5 returned supplier business errors. Focused Takeoff MH still returned six null refreshed refs.
- [ ] Remaining supplier errors: Takeoff BS no-valid-fare, Triplover BG requested-class unavailable, and Triplover TK multicity duplicate-key `DAC->IST`. These results do not verify FareRules, acceptance, Book/Cancel/PNR or ticketing, or close other pending requirements.
- Evidence and next action: [supplier fix recheck](docs/evidence/SUPPLIER_FIX_RECHECK_2026-09-09.md). Resolve refreshed-reference semantics and remaining supplier errors before claiming full flow readiness; no tax rewriting or Search-reference fallback was introduced.


### Supplier-confirmed RePrice reference contract — 2026-09-10

- User relayed the supplier contract: Search response supplies item code and segment refs; RePrice request uses those Search references; RePrice response supplies item code and price code; Book request uses the RePrice item/price codes. RePrice response segment references are not required.
- This supersedes earlier notes treating null RePrice segment references as a supplier failure or an unresolved mandatory-reference contract. The prior 422 was caused by our integration assumption.
- [x] Removed mandatory response segment-reference validation. Null response segment fields remain unchanged; selected Search refs still select and validate the requested itinerary and are persisted separately. Required transaction/item/price references, pricing, ownership, expiry and itinerary checks remain.
- [x] Existing Book payload already uses refreshed transaction/item/price references and does not send segment refs; no booking execution is authorized by this clarification.
- User authorized fresh Search → RePrice verification only and explicitly prohibited booking. Live results are recorded separately after verification.
- [x] Fresh production read-only recheck: all 12 matrix Searches plus one focused Search passed without partial failures; 35/38 RePrices passed, including all roundtrip/multicity samples. Twelve successful RePrices retained null segment refs. Triplover 6E and TK both passed twice; focused Takeoff BS/MH passed.
- [ ] Three one-way supplier fare errors remain (Takeoff BS, Triplover BG, Firsttrip BG). No missing-segment-reference errors occurred; previous Triplover TK duplicate-key error was not reproduced. No booking or deployment occurred.
- Validation: ordinary tests, disposable PostgreSQL tests, formatting and strict Clippy passed; independent audit checked 7,281 raw Search offers, 7,269 selling offers and 35 successful RePrices. Full scope and evidence: [contract recheck](docs/evidence/REPRICE_CONTRACT_RECHECK_2026-09-10.md).


### Customer-selected same-airline alternatives through prebooking — 2026-09-10

User explicitly requested same-airline alternative selection through the step before booking; no live booking or direct issue is authorized. This is a backend/API workflow; a separate frontend remains out of scope.

- [x] Customer may choose another retained offer on the same plating airline after one fare is rejected. Use that offer's own item and selected segment references; no silent airline/supplier change, no automatic purchase. Existing Search/RePrice endpoints support this selection without a new alternative endpoint.
- [x] Evidenced unavailable-fare messages map to 409 `FARE_UNAVAILABLE`; evidenced supplier-session expiry maps to 410 `SUPPLIER_SESSION_EXPIRED`. Unknown business errors remain generic 502. A fare rejection does not invalidate all offers of that airline.
- [x] Migration 0011 persists per-offer `reprice_required`: classified fare/session rejection blocks old accepted quotes from reacceptance and new booking. A fresh successful RePrice clears the flag and creates a new version. Other same-airline offers remain independent.
- [x] FareRules now accepts the same exactly-one-direction-per-route selection as RePrice and forwards only selected original references. Partial/mixed/ambiguous selections remain rejected.
- [x] Mock/database coverage verifies 6E Q → rejection → customer selects 6E V → FareRules/RePrice → local acceptance, null response segment references, no automatic alternate/supplier call, old-price rejection, successful recovery, session expiry, and zero bookings. `bookable=false` acceptance is tested as local-only; existing direct-issue Book guards and rejected-quote Book guard are mock-tested.
- [x] Fresh live 6E multicity test: Search 200 (528 offers), N and M fare RePrices both 200, customer-selection simulation accepts only M locally (200). Two revisions, one accepted, zero bookings. Independent markup audit passed for 528 Search offers and both RePrices.
- [ ] Live FareRules for both 6E fares returned supplier `Fare display key not found for Indigo` (public 502). This remains a supplier-contract/availability limitation; no claim of full FareRules acceptance is made.
- Validation: ordinary suite, disposable PostgreSQL suite, formatting and strict Clippy. No Book/Cancel/NewTicket/direct issue, deployed database migration, or deployment occurred. Other Step 3/4/6/7/8 gaps remain open.
- Contract: [prebooking flow](docs/PREBOOKING_FLOW.md). Evidence: [same-airline prebooking validation](docs/evidence/SAME_AIRLINE_PREBOOKING_2026-09-10.md).
- FareRules follow-up: direct supplier-adapter verification reproduced the IndiGo key error in four fresh FareRules calls (oneway/multicity, before/after successful RePrice). BG oneway FareRules passed as a control. Documented fields and selected original references matched; no missing documented input was found. Provider-specific handling or an undocumented requirement remains to be clarified, not a conclusively attributed bug. [Investigation](docs/evidence/FARE_RULES_CONTRACT_INVESTIGATION_2026-09-10.md).


### FareRules upstream-error handling — user approved 2026-09-10

- [x] FareRules business/transport failure now returns 502 `UPSTREAM_FARE_RULES_ERROR`; raw supplier details remain private. Deadline timeout remains 504.
- [x] FareRules failure does not mark the offer unavailable or invalidate its quote. Customer may continue to RePrice; unavailable rules must be shown as unavailable, not fabricated. Successful RePrice still needs explicit local price acceptance.
- [x] Regression coverage verifies supplier-business and transport error mapping, unchanged offer pricing eligibility and continued RePrice/acceptance. No live booking/issue or deployment.

### Prebooking release deployed — 2026-09-10

- [x] Selected-direction FareRules/RePrice, nullable response segment references, rejected-quote protection and FareRules error handling deployed as application commit `1233c5622e7f6f58c7027f9f41667e8ebc33b2cd` through GitHub Actions run `34446909044`.
- [x] Local checks and remote CI passed; deployment applied migrations 0010/0011 through the existing backup/migration helper. Public HTTPS health/docs/OpenAPI, new contract descriptions and unauthenticated access rejection verified.
- Earlier notes saying these changes were local-only describe their implementation milestones and are superseded by this release result. No supplier booking/issue or authenticated supplier-flow test was performed for deployment verification.
- [ ] Step 4 aggregate Search summaries/filters and other documented Step 3/4/6/7/8 gaps remain open. Next implementation focus: summaries derived from retained selling offers.
- Evidence: [prebooking release verification](docs/evidence/PREBOOKING_RELEASE_2026-09-10.md).


### Aggregate Search summaries — 2026-09-10

- [x] Existing summary fields derive from retained selling offers after supplier selection/markup: offer count, represented supplier count, net selling range, associated AIT, independently aggregated gross range, per-plating-airline counts/minima and distinct direction stops.
- [x] `X-Search-Summary-Scope: retained-selling-offers` replaces the historical first-successful-supplier limitation for these known fields. Unknown fields and missing/null shape are preserved; airline templates retain their source fields. Offer markup/reference behavior is unchanged.
- [x] Empty responses clear numeric summaries/arrays. Unsupported populated metadata returns `SUPPLIER_SUMMARY_UNSUPPORTED` and rolls back Search/offer persistence. Numeric totalPages describes the single complete response; null remains null and no pagination endpoint is added.
- [x] Unit/fixture checks cover cross-supplier airlines, exact decimals beyond floating-point precision, passenger-count AIT/gross, return/multicity stops, empty/null/missing data and atomic errors. Database integration checks cover all seven supplier subsets, post-deduplication counts, partial failure and rollback.
- [ ] Broader canonical equivalence, complex scoped markup, branded/multiple-component fares and remaining booking/ticketing acceptance are still open. This update does not complete Step 4 overall.
- [x] Fresh production read-only Search: all 12 one-way/roundtrip/multicity × all/individual supplier scenarios passed without partial failures. Independent Decimal audit verified summaries and markup across 7,207 retained offers. No booking/issue calls.
- Contract: [Search summaries](docs/SEARCH_API.md). Evidence: [summary verification](docs/evidence/SEARCH_SUMMARY_2026-09-10.md).

- [x] Summary aggregation deployed as application commit `ee3de025cf5b5069b211051337059b3218c2875d`, GitHub Actions run `34448160884`; all CI/build/deploy jobs passed. Public HTTPS health, served summary contract and unauthorized access rejection verified. No new migration or booking/issue execution.


### Search memory and persistence optimization — 2026-09-10

- [x] Offline replay profiled a 2,177-returned-offer roundtrip workload at one/four concurrent Searches. Three runs per version showed about 48%/44% lower peak replay-process RSS and 15%/18% lower median local HTTP latency after removing full inventory/response copies and batching offer inserts (64 rows maximum).
- [x] Offer snapshots, pricing, selection and references remain unchanged. All batch inserts share one transaction. Tests cover 130 rows, failure in a later batch with full rollback, empty result and existing workflow checks.
- [x] Added safe phase/count logs and a reusable local-capture-only load example. No supplier traffic or live booking/issue was required.
- [ ] The 23.3 MiB sample response and snapshot storage size remain unchanged. Compression, expiry cleanup policy, admission controls and controlled production-equivalent load validation are not completed or implied by these local measurements.
- Evidence and environment limits: [Search performance](docs/evidence/SEARCH_PERFORMANCE_2026-09-10.md).

- [x] Memory/copy and batch-persistence optimization deployed as `24ba2632bd77a6d70b98033c82c314668d6b1b10` through GitHub Actions `34449963975`; all checks/build/deploy passed, public HTTPS health/docs/auth checks verified. Production concurrent capacity remains unmeasured.


### Critical priority: further optimization without losing offers — user directive, 2026-09-10

- Further optimization is a high-priority requirement before increasing production load. Prior memory/batch work is not evidence that optimization or VPS capacity validation is complete.
- Optimization must not truncate/top-N/filter away valid returned offers, fare classes, airline/route options, or bypass supplier-compatible field/precision preservation. Existing explicitly approved equivalent-fare lowest-supplier selection remains the business baseline; do not broaden deduplication merely to save memory.
- Compare `/Users/ashifbabu/Projects/tripfeels-backend` implementation against this project before choosing further techniques. Reuse suitable techniques, not the other project's pricing, retention, reference, security or response policies by assumption.
- Distinguish wire bytes, live/peak process memory, repeated-search work, database growth and concurrent capacity. Compression must round-trip losslessly; caching/sharing must preserve client ownership, current pricing context, supplier enablement epochs and original reference expiry. RePrice/acceptance/booking state must remain isolated.
- Requested scope for this review: inspect, compare and report feasible reductions without removing offers; no new runtime optimization or deployment is implied by the comparison itself.

- Review completed: [Tripfeels comparison and prioritized recommendations](docs/evidence/TRIPFEELS_OPTIMIZATION_COMPARISON_2026-09-10.md). Exact 2,177-offer payload gzip proof reduced 24,446,992 bytes to 1,148,213 bytes at level 6, with byte-identical decompression; no runtime change/deployment.


### Offer-preserving gzip implementation — 2026-09-10

- [x] User authorized proceeding carefully after the comparison. Added negotiated gzip for eligible responses of at least 1 KiB, using the fastest level. No offer truncation, selection, pricing, persistence or expiry changes.
- [x] Exact response-byte equality, encoding negotiation, small health/auth errors and existing gzip-decoded Search integration assertions pass. Local one/four-concurrent replay retained and persisted all 2,177 baseline offers per request; response transfer fell from 24,446,992 bytes to about 2,656,000 bytes (89.1%).
- [ ] Remaining clone elimination, expiry cleanup policy, appropriately isolated shared snapshots/caching and VPS capacity validation remain open. Gzip does not shrink decoded inventory or database snapshots.
- Evidence: [gzip verification](docs/evidence/SEARCH_GZIP_2026-09-10.md). This milestone does not complete the broader optimization requirement.

- [x] Gzip deployed as `fca1e85cf8aaa638122704c79dc26cfad758e78b`, GitHub Actions `34459503903`; all CI/build/deploy jobs passed. Public HTTPS gzip/identity decoded bytes match exactly, encoding negotiation and health/auth checks pass. No new migration or supplier calls were needed for this milestone.


### Borrowed validation and comparison — 2026-09-10

- [x] Removed full-offer copies used solely for zero-markup validation and equivalence-key construction. Original field exclusions/key bytes, coverage rejection, price precision, supplier winners and cabin conflicts remain unchanged.
- [x] Differential tests against frozen prior implementations, existing unit/fixture checks and disposable PostgreSQL integration pass. Final offline replay preserves 2,177 returned/persisted offers per Search and unchanged decoded size/business fingerprints.
- [x] Final three-run local medians: preparation 93→20 ms (one request) / 158→21 ms (four concurrent); process CPU about 10%/14% lower. Full-body latency is mixed and peak RSS about 3% higher, so this is not claimed as a RAM reduction or VPS capacity improvement.
- [ ] Peak-memory profiling/reduction, expiry cleanup policy, safely isolated sharing/caching and production-equivalent capacity verification remain incomplete.
- Evidence: [borrowed validation and comparison](docs/evidence/SEARCH_BORROWED_VALIDATION_2026-09-10.md).

- [x] Borrowed validation/comparison deployed as `e507ef9046ff01d0edcd73153fc02806107982c2`, GitHub Actions `34461353085`; all CI/build/deploy jobs passed. Post-activation public HTTPS health/auth/gzip checks passed. No new migration or live supplier call was required.


### User workflow and temporary Search retention — 2026-09-10

- Explicit user directive: complete and verify local work, then obtain the user's permission **before committing or pushing**. Do not automatically deploy unapproved changes. This supersedes the previous automatic commit/push workflow.
- User questioned retaining temporary Search data beyond 15 minutes. Existing Search/reference validity remains 10 minutes. A 15-minute cleanup policy for unused temporary inventory is a proposed next step, not implemented by the current memory change.
- `flight_reprices` and `flight_bookings` reference selected `flight_offers`, which reference `flight_searches`. Cleanup must preserve dependent selected-offer/booking data, concurrent reads/booking safety and the intended expired-reference response. Do not delete all old Search rows or cascade business records merely by age.


### Peak-memory reduction — verified locally, 2026-09-10

- [x] Phase profiling identified overlapping original/selling trees as the main sampled memory increase. Search now encodes original snapshots in 64-offer batches and reuses owned offer trees for selling data, while retaining every baseline offer.
- [x] Final local three-run medians: peak replay RSS 554.64→361.66 MiB at one request and 1,824.77→1,061.05 MiB at four concurrent requests (about 35%/42% lower). No VPS capacity claim.
- [x] Exact projection comparisons, complete original/selling snapshot checks, decimal lexeme preservation and rollback after 130 attempted inserts with late summary failure pass, alongside existing tests.
- [x] User subsequently authorized adding cleanup and then committing/pushing this combined release. Cleanup policy and verification are recorded below.
- Evidence: [Search memory lifetimes](docs/evidence/SEARCH_MEMORY_LIFETIMES_2026-09-10.md).


### Temporary Search cleanup and current release authorization — 2026-09-10

- User instruction: “Cleanup … যোগ করার পরে push দিয়ো.” This authorizes committing/pushing the verified memory optimization plus cleanup, overriding the earlier approval hold for this release only.
- [x] Serve-mode cleanup every 30 seconds removes expired, unreferenced offers aged at least 15 minutes and empty expired Search headers. RePrice/booking-linked offers and business records are retained; validity remains 10 minutes.
- [x] Bounded 512-row transactions, SKIP LOCKED, instance coordination, timeouts and per-tick budgets protect foreground work. Backlog/locks may delay deletion past eligibility.
- [x] Owner-scoped IDs (no payload) retain 410 behavior for 24 hours after deletion; foreign/unknown references and expired markers return 404.
- [x] Added tests for batch limits, rollback, locks, protected records, fresh/grace/valid-parent cases, client isolation, marker expiry and actual scheduler/shutdown.
- Evidence: [cleanup policy and verification](docs/evidence/SEARCH_CLEANUP_2026-09-10.md).


- [x] Combined memory optimization and temporary Search cleanup deployed as `b739a579fce5792b1ef4c80d57dbe434ccbbbad4`, GitHub Actions `34464006037`; CI/build/deploy and migration 0012 activation succeeded. Public HTTPS health/auth/gzip checks passed. Production cleanup counts were not inspected because the deploy account lacks system-journal access; actual scheduling and cleanup policy passed local/CI integration tests.


### VPS observation and controlled capacity verification — 2026-09-10

- User authorized proceeding with cleanup observation and isolated concurrent replay verification. Future commit/push still requires explicit permission.
- [x] User installed the fixed read-only observation helper from a root terminal. Production journal confirms the 15-minute/30-second cleanup worker startup; aggregate inspection shows no eligible backlog and currently empty Search inventory, so a production deletion event is not claimed.
- [x] Controlled VPS replay completed at concurrency 1, 2 and 4, three samples each, with private disposable PostgreSQL and readiness sampling. All 2,177 baseline retained offers, decoded size and prior business fingerprint preserved. Median HTTP times: 6.192 / 12.363 / 24.265 seconds; replay peak RSS: 664.55 / 827.48 / 1,404.84 MiB. These capped offline measurements do not establish general production capacity. The original 7,207 count totals 12 scenarios; it was not a single-response workload.
- [x] Initial 256 MiB test PostgreSQL cap caused a four-request OOM failure; all final levels passed with a 512 MiB test DB cap. Production remained active. Temporary VPS files/cluster removed; read-only observer remains installed.
- [ ] Evaluate bounded Search admission/queue and database persistence memory on the largest captured workload before choosing production limits. User subsequently authorized committing/pushing the observer scripts and verification documentation; further runtime changes/deployments remain outside that approval.
- Evidence: [controlled VPS replay](docs/evidence/SEARCH_VPS_CAPACITY_2026-09-10.md).


### Bounded Search admission — verified locally, awaiting commit/push approval

- User authorized proceeding with the next optimization step. Complete local work and verification, then ask before committing/pushing runtime changes.
- Capture clarification: the earlier 7,207 offers total 12 Search scenarios; largest retained response is 2,177 offers. Do not describe 7,207 as a demonstrated single-response fixture.
- [x] Configurable admission after authentication/permission/request validation and before supplier/configuration snapshots. Updated defaults after the user specified 10-12 simultaneous customers: four active Search responses, eight waiting requests, user-approved two-second maximum queue wait. Per-process guardrails, not a production capacity guarantee.
- [x] Capacity remains held through JSON serialization and gzip response-body consumption/drop. Busy requests return whole-request 503 SEARCH_BUSY with Retry-After: 1, without supplier calls or offer persistence; no offer truncation or cache/reference sharing.
- [x] Formatting, all-target clippy, 42 unit/foundation/fixture tests and the real PostgreSQL integration pass. Nine local four-arrival replays preserve all 2,177 offers per request and the prior fingerprint/decoded size. Two active slots reduce median replay RSS from 1,051.55 to 596.30 MiB (43.3%), with complete-wave time increasing from 2.654 to 3.421 seconds; comparison uses a 30-second queue budget and is not a default-policy/VPS capacity claim.
- [x] All disposable admission/replay databases removed. No supplier network calls, migrations, commit, push or deployment performed for this local change.
- Evidence: [Search admission](docs/evidence/SEARCH_ADMISSION_2026-09-10.md).


### Customer burst requirement - local follow-up

- User expects 10-12 customers to Search simultaneously. The initial two-active/four-queued/two-second draft is superseded by four active slots, eight queued slots and a user-approved two-second maximum wait. This admits twelve requests into active/waiting capacity, not twelve expensive pipelines at once or a guarantee of success under every supplier delay.
- Offline example now accepts 1-12 concurrent arrivals. Compare four and six active workers using the same captured 2,177-offer response; retain every baseline offer.
- The earlier 240-second Nginx proposal is withdrawn after the user capped queue wait at two seconds; keep the existing reviewed 150-second timeout. No live config change or commit/push is authorized by this local implementation step.

- [x] Final twelve-arrival local comparison: three samples each at four/six active slots. All 72 responses preserve every baseline offer. Median replay RSS 1,282.75/1,805.28 MiB; complete-wave times 5.796/6.155 seconds. Final comparison ran after builds/tests finished; preliminary overlapping measurements were excluded.
- [x] Three historical replay samples with the intermediate defaults (4 active/8 queued/90000 ms wait) pass for twelve arrivals each. Forty-three unit/foundation/fixture tests, clippy, formatting and PostgreSQL integration pass. This is not a VPS capacity promise.
- Evidence: [twelve-arrival policy](docs/evidence/SEARCH_BURST_2026-09-10.md).


### Final queue deadline - explicit user correction

- User rejected the 90-second proposal and requested at most one second. Default and hard upper bound are now 1000 ms; environment and programmatic configuration reject larger values. Active/queued counts remain four/eight.
- A queued request starts immediately when capacity opens, otherwise returns SEARCH_BUSY when its one-second wait expires. This deadline does not limit supplier execution after admission. Historical longer-budget burst success is not a promise of twelve successful requests under this short wait policy.
- No commit, push, deploy or live Nginx change is authorized by this correction.


### Final user approval of the two-second queue setting

- User superseded the one-second instruction with "2 SEC FINAL". Final default and hard maximum: 2000 ms; four active and eight queued slots remain. This approves the local setting only; commit/push/deploy still require permission.
- Earlier one-second and 90-second proposals are superseded. Existing 150-second Nginx timeout stays unchanged.

- [x] Final two-second default replay: three runs of twelve arrivals, each with eight successes and four SEARCH_BUSY responses. All successful responses preserve 2,177 offers/fingerprint/decoded size; 17,416 persisted offers per run. Forty-three tests, clippy, formatting and release build pass. Local measurements only; no production update or commit/push.


### Controlled VPS validation of final admission policy

- [x] User authorized isolated VPS validation before commit/push/deploy. The tested source tar and local runtime file hashes match. One smoke Search and three twelve-arrival waves completed; no supplier network calls or production data writes were used.
- [x] Each twelve-arrival wave: four successes, eight SEARCH_BUSY responses, 8,708 persisted offers; all successful responses retain the baseline fingerprint/decoded size. Median burst replay RSS 1,519.65 MiB, successful HTTP 23.532 s, busy HTTP 3.099 s under explicit CPU/RAM caps. A two-second admission timer is not a two-second end-to-end response guarantee.
- [x] All 551 sampled production health checks passed; no replay/PostgreSQL OOM/max events observed. Test cluster, captures and build files removed from VPS. Production PID unchanged; no commit/push/deploy or Nginx change performed.
- [x] User explicitly authorized commit/push/deploy after reviewing the result. This bounded configuration does not establish that all twelve simultaneous customers can succeed; eight requests were rejected per measured wave. Further capacity work remains separate.
- Evidence: [VPS twelve-arrival replay](docs/evidence/SEARCH_VPS_BURST_2026-09-10.md).

### Admission release authorization

- User explicitly requested "Commit/push/deploy KORO" after the VPS results and two-second timer limitations were explained. This supersedes the earlier pending release permission. Release scope is four active Search requests, eight queued slots and a 2000 ms admission wait; existing 150-second Nginx timeout remains.

### Admission release completed

- [x] Authorized application commit `7c24ab0818f8315cba113e07d75143d2952178de` pushed to main and deployed successfully through CI on 2026-09-10 at 18:11 Asia/Dhaka. All checks/build/deploy jobs passed; eight independent public HTTPS checks passed afterward. Cleanup worker retained 15-minute retention / 30-second interval, with zero observed eligible backlog.
- Evidence and measurement limits: [admission deployment](docs/evidence/SEARCH_ADMISSION_DEPLOY_2026-09-10.md).

### Database persistence profiling and smaller batches — 2026-09-10

- User authorized proceeding with the next optimization step. Local implementation/verification only; commit/push still require permission.
- [x] Separated projection, SQL encoding/execution, commit timing and batch count in safe aggregate Search logs. Local comparison uses the same timing instrumentation for both variants.
- [x] Reduced INSERT batches from 64 to 16 offers while retaining one atomic transaction and all original/selling/reference data. No migration, admission, expiry or pricing change.
- [x] Twelve alternating local replays (three per variant at concurrency one/four) preserve all 2,177 offers and prior business fingerprint/decoded size per response: 30 successful responses, 65,310 persisted offers. At four concurrent requests, median sampled summed PostgreSQL RSS 573.86→346.30 MiB, SQL execution 1,048→848 ms, HTTP 1,537→1,338.5 ms. Replay-process peak RSS increases 1,080.14→1,155.30 MiB; no application-RAM or storage-reduction claim.
- RSS sums include shared-page double counting and are not cgroup/unique physical memory. These local socket measurements do not establish VPS capacity or twelve-customer success. More SQL statements may cost more with network database latency.
- [ ] Constrained VPS comparison of smaller batches/database cgroup memory and the final twelve-arrival admission policy remains pending. Shared snapshot/cache isolation and other Step 3/4/6/7/8 gaps are unchanged.
- Evidence and verification: [persistence profiling](docs/evidence/SEARCH_PERSISTENCE_2026-09-10.md).
- [x] Formatting, strict all-target Clippy, all 43 ordinary tests and disposable PostgreSQL integration (36.98 seconds) passed, including complete snapshots/reference ownership and late-batch/summary rollback. Profiling cluster and integration database removed; no supplier calls, production writes or release performed.

### Search response memory — 2026-09-10

- User requested continuing optimization and explicitly deferred push/deployment. The previous smaller-batch change remains local.
- [x] Profiled four-request memory phases. The earlier 7% increase describes the combined replay process, including client parsing, and was not consistently reproduced; it is not proof of a standalone API RAM regression.
- [x] Search response serialization now releases each offer tree after encoding it. Axum still buffers the whole response before returning it; all fields, exact numbers, offer order, database atomicity and admission permit lifetime remain intact.
- [x] Twelve paired local replay runs preserve 2,177 offers and exact decoded size/prior business fingerprint in every response. Median replay RSS 353.50→335.92 MiB (one request) and 1,250.28→1,133.53 MiB (four). Four-request HTTP median 1,326→1,390.5 ms; no speedup or isolated API/VPS memory guarantee is claimed.
- [x] Exact-byte tests against all three supplier fixtures and unknown/precise-number shapes, formatting, strict Clippy and all 45 ordinary tests pass. Full disposable PostgreSQL integration passed in 36.97 seconds; temporary database and profiling cluster removed.
- [ ] Shared snapshot/cache investigation and constrained VPS capacity verification remain separate; no policy inference, supplier traffic, commit, push or deployment is introduced.
- Evidence: [response memory verification](docs/evidence/SEARCH_RESPONSE_MEMORY_2026-09-10.md).

### Persistence/response optimization release authorization — 2026-09-10

- User explicitly requested completing Git push and remaining database/deployment work. This supersedes the prior push/deploy deferral for the verified 16-row SQL batches, aggregate timing logs and consuming Search response serializer.
- Release uses the existing CI/VPS activation workflow, including backup, migration/checksum validation, grants and health checks. No new migration or production test data upload is needed; no supplier Search/Book/Issue is part of release verification.
- Shared caching and expanded VPS capacity verification remain separate; deployment does not establish twelve-request success.

### Persistence/response optimization release completed — 2026-09-10

- [x] Application commit `b08b34d8dbc223f3c37fb75d64db3f813f19ff6d` pushed to main and deployed through successful CI run `34482728952` at 19:34:45 Asia/Dhaka. All checks/build/deploy jobs passed.
- [x] Existing deployment backup/migration/grants sequence completed; no new schema migration or local-data upload was required. Eight independent post-activation HTTPS checks passed; active service PID 75311 and cleanup worker startup verified through the read-only observer.
- Evidence: [release verification](docs/evidence/SEARCH_MEMORY_DEPLOY_2026-09-10.md). Local-only/deferred-release descriptions above are historical and superseded for these two changes; cache/capacity gaps remain separate.

### Controlled VPS comparison of memory release — 2026-09-10

- User authorized isolated VPS verification of four/twelve simultaneous Searches using captured responses, without supplier traffic or production data changes.
- [x] Compared current application `b08b34d` with the prior admission binary under identical replay/PostgreSQL caps, three alternating samples per version at each level plus one candidate smoke test. Candidate source/upload/binary hashes verified.
- [x] All four-arrival waves: four successful responses. All twelve-arrival waves: four successes/eight `SEARCH_BUSY` in both versions. All 49 accepted successful responses retain 2,177 offers, exact decoded size and prior fingerprint; 106,673 persisted offers across disposable databases.
- [x] PostgreSQL median cgroup peak fell 409.39→209.34 MiB at four arrivals and 414.39→210.59 MiB at twelve (about 49%). Replay RSS changed −2.1%/+2.5%; successful HTTP median increased 7.6%/2.1%, and replay CPU increased 25.1%/11.5%. No consistent VPS application-RAM improvement, speedup or twelve-request all-success capacity claim.
- [x] A monitor stop between SSH sessions was detected; six preliminary unmonitored burst runs were excluded and repeated with continuous per-wave monitoring. Accepted-run timestamp coverage verified. All 685 collected health samples passed; no accepted replay/PostgreSQL OOM/max events. Test cluster/files removed; production remained active with PID 75311 and final live/ready passed.
- The previously pending constrained comparison is now completed for this workload and these caps. Keep admission defaults unchanged. Further CPU-cost isolation/intermediate-batch comparison and shared snapshot/cache feasibility remain separate; broader production capacity remains unproven. No runtime edits, commit/push or deployment in this verification step.
- Evidence: [VPS memory comparison](docs/evidence/SEARCH_MEMORY_VPS_2026-09-10.md).
- Subsequent user decision: defer further CPU/batch-size optimization and capacity testing until VPS replacement. Current deployed code and admission settings stay unchanged. User authorized committing/pushing these verification documents only; use `[skip ci]` to avoid a new deployment.


### Step 6 public PNR lookup and deadline persistence — 2026-09-10

- [x] Added `POST /api/pnr` with the documented six-field casing and platform-owned Search/offer/price/booking UUIDs. Requires booking permission; foreign booking IDs and mismatched references are rejected before supplier traffic. Existing booking servicing remains independent of Search/new-booking enablement.
- [x] Public PNR and existing reconciliation use the same saved supplier references. Corrected field-specific identity projection when supplier reference strings overlap; preserve nullable references and the documented `bookingRef` PNR mirror.
- [x] Validate PNR/status/echoed references, atomically persist evidence and redacted audit, and update the latest raw ticketing deadline. Missing/invalid latest deadlines clear old authority; failed responses do not overwrite deadlines. Database dispatch timestamps prevent older overlapping observations from overwriting newer persisted ones. Supplier timezone remains unverified.
- [x] PNR reads never Book/Cancel/Issue and never resolve an uncertain booking automatically. Local status/idempotent Book output remains the original outcome; current supplier status/deadline is exposed through PNR. Manual-resolution header remains true for unresolved bookings, non-Booked supplier status or ticket-number evidence.
- [x] Formatting, strict all-target Clippy and all 45 ordinary tests pass. Full disposable PostgreSQL integration passed in 37.07 seconds, including public PNR ownership/reference isolation, servicing controls, colliding/null reference projection, invalid supplier responses, deadline clearing and stale-observation protection. Disposable database removed after verification.
- No supplier network calls, real booking/ticketing, schema migration, commit/push or deployment performed. Actual Hold/Issue validation remains restricted to Triplover UAT under the latest user instruction.
- [ ] Step 6 remains partial: audited manual resolution, Cancel/idempotency and full lifecycle/supplier acceptance remain unfinished. This update implements the next PNR increment, not the whole booking lifecycle. Step 7 ticketing and other documented coverage gaps are unchanged.
- Contract: [Booking and PNR API](docs/BOOKING_API.md).

### Admin reconciliation and coding-free screen — 2026-09-10

- User approved the proposed Admin screen and manual reconciliation. This narrowly supersedes the initial frontend exclusion for reconciliation only; full administrative dashboard remains outside scope.
- [x] Bengali `/admin/reconciliation` screen: existing human-admin login, unresolved/resolved lists, status recheck, supplier-confirmed outcome form and immutable decision history. Admins do not edit code or enter API reference/version fields.
- [x] Protected Admin list/detail/recheck/resolve APIs; explicit evidence/reference/confirmation, optimistic version plus row lock, atomic audit and manual outcomes (`held`, `not_created`, `issued`, `cancelled`). Client tokens are rejected; Admin sessions remain separate from commercial API credentials.
- [x] Pending requests younger than five minutes cannot be manually resolved. Delayed supplier completion preserves new evidence, reopens review and retains earlier decisions. Original idempotency keys never redispatch Book after manual resolution.
- [x] Migration 0013 adds immutable history/late-outcome records, manual state and verified-PNR evidence flag. No passenger payload is exposed in the screen; private manual evidence stays out of client status and operational audit metadata.
- [x] All 46 ordinary tests and full disposable PostgreSQL integration pass (final integration: 37.00 seconds), with strict Clippy, formatting and JavaScript syntax validation. Browser interaction/visual QA and live UAT acceptance are not claimed.
- Local implementation only; no working/production database migration, supplier Book/Issue/network call, commit/push or deployment. Actual supplier booking validation remains Triplover UAT only.
- [ ] Step 6 remains partial: Cancel/idempotency, complete servicing acceptance, private-data production protection/retention and remaining lifecycle coverage. Step 7 ticketing remains pending. Manual resolution records a verified human outcome, not an automatically verified held payload or ticketing authorization.
- Contract and evidence: [Booking/Admin API](docs/BOOKING_API.md), [verification](docs/evidence/ADMIN_RECONCILIATION_2026-09-10.md).

### Admin browser verification and public Triplover UAT flow — 2026-09-10

- [x] User authorized browser checks and UAT continuation. Actual browser verification passed for login/error handling, selection/status recheck, required fields, synthetic evidence submission, resolved queue/history, missing-reference guidance, logout and reload. Corrected heading/help text to follow the resolved filter; reverified the final screen. No mobile/all-browser coverage claim.
- [x] Corrected one non-comment heading in local `.env` that prevented parsing. Preserved host/credential assignment values. UAT runner now rejects dotenv parse failures explicitly and asserts exact HTTPS UAT hosts before network access.
- [x] Public Triplover UAT Search (36 offers), RePrice and local acceptance passed. Exactly one Book was dispatched using the previously authorized UAT passenger input; supplier returned a duplicate-booking business failure without PNR. Public HTTP 202 `outcome_unknown` and identical same-key replay were verified; no second supplier Book.
- [x] Client reconciliation and Admin recheck correctly returned 409 `MANUAL_RECONCILIATION_REQUIRED` without guessed references. A separate read of the prior UAT hold's original PNR references again returned `Record locator not found`. No real UAT manual decision, Cancel or Issue was performed.
- [ ] Successful public UAT Hold → live PNR → reconciliation acceptance remains open. Supplier support must clarify duplicate/status/reference behaviour; failure output does not prove booking absence or authorize a blind retry. Step 6 remains partial.
- [x] 46 ordinary tests, strict Clippy, formatting and JS syntax checks pass. Synthetic browser server stopped and disposable databases removed; UAT booking/audit data preserved in a private database archive and private request/response evidence.
- No production database update, production Hold/Issue, commit/push or deployment. Browser/UAT-pending statements in the prior update are superseded only for the coverage actually verified here.
- Evidence: [browser and UAT verification](docs/evidence/ADMIN_BROWSER_UAT_2026-09-10.md).

### Complex route/airline markup review — 2026-09-10

- User deferred the proposed UAT-status/Cancel work and authorized reviewing the next complex markup increment.
- [x] Reviewed shared Search/RePrice scope matching and 18 saved supplier Search responses (7,104 repeated offer occurrences). Real connecting, mixed-carrier, return and multicity cases require an explicit whole-fare matching policy. Existing All/All guard regression passes.
- [x] Prepared concrete options, examples and a proposed first-requested-route + plating-carrier convention, keeping directional routes, existing audience priority and one markup per passenger fare.
- [ ] User decision on that proposed convention remains required under §5.7. No general complex scoped matching is enabled; the proposal is not an approved business rule. Branded/multiple-component coverage and other Step 4 gaps remain separate.
- No supplier calls, runtime pricing changes, database writes or release. Report: [complex markup decision](docs/evidence/COMPLEX_MARKUP_DECISION_2026-09-10.md).

### Return scope approval and conditional per-leg calculation — 2026-09-10

- User approved return route matching: DAC→SIN→DAC uses the applicable DAC→SIN rule as one journey. A separate SIN→DAC one-way does not reverse-match that rule; a SIN→DAC specific rule must be configured for that scope. Existing audience priority and broader fallback remain unchanged.
- User requests per-leg markup **if the supplier actually supplies separate passenger-type fares for each leg**. This condition supersedes the earlier blanket per-segment prohibition only for verified leg/pax fare coverage. Do not infer leg prices by splitting totals or treating a booking component as a leg.
- [x] Offline review of 7,104 Search offers and 33 populated RePrice fares found whole-journey passenger fares, one booking component each, and exact passenger-count sums matching offer totals. Directions/segments do not contain separate leg/pax monetary fares in these samples. Current whole-journey passenger calculation remains applicable to this coverage.
- [ ] Return scoped matching implementation remains pending. Mixed-airline carrier selection and multicity route selection are not approved by this clarification; the earlier proposal remains a proposal for those cases. Future actual leg/pax breakdowns need verified mapping and reconciliation before enabling per-leg calculation.
- Evidence and a numeric return example: [passenger/leg fare review](docs/evidence/PASSENGER_LEG_FARE_REVIEW_2026-09-10.md). No supplier traffic or runtime pricing change.

### Approved return markup implemented locally — 2026-09-10

- [x] User authorized proceeding after the fare review. Shared Search/RePrice matching now maps exact A→B→A returns to the outbound A→B rule. A separate B→A one-way does not reverse-match it; established fallback still applies.
- [x] Validate all alternatives, route endpoints and segment continuity. Return airline-specific matching requires consistent plating/segment carrier and explicit non-codeshare evidence. Airline-independent rules can use the return route without choosing a mixed/codeshare governing airline. Open-jaw/multicity scoped matching and unresolved carrier-specific cases remain guarded.
- [x] Current whole-journey passenger fares receive one markup; no per-leg multiplication or fabricated split. Mock public Search → RePrice → acceptance confirms the same winning outbound price, with both directional rules active.
- [x] All 49 ordinary tests, strict Clippy, formatting and full disposable PostgreSQL suite pass (37.28 seconds). Test database removed. No new migration, working/production database update, supplier traffic, commit/push or deployment.
- The prior return-implementation-pending note is superseded within this coverage. Evidence: [return markup implementation](docs/evidence/RETURN_MARKUP_IMPLEMENTATION_2026-09-10.md).

### Multicity first-route rule — user approved, 2026-09-10

- After rechecking multicity whole-journey fares, the user selected the first route for rule matching. For DAC→SIN, SIN→BKK, BKK→DAC, use DAC→SIN. "First segment" here refers to the first requested journey route, not its first connecting flight segment. Later-route rules are not added, averaged or used as the first available match.
- [x] Shared Search/RePrice matching now covers return and multicity requests with two or more routes. All route alternatives/endpoints and connection continuity are validated. Existing audience/scope fallback applies when the first route has no matching specific rule; no applicable rule remains `PRICING_CONFIGURATION_ERROR`.
- [x] Markup applies once to each whole-journey passenger fare, then passenger counts aggregate. Previous proposals for averaging different route percentages are superseded and were not implemented. Mixed/codeshare airline-specific decisions and actual separately priced leg/passenger coverage remain separate.
- Tests and release boundary: [multicity implementation](docs/evidence/MULTICITY_MARKUP_IMPLEMENTATION_2026-09-10.md).

### First-segment airline — explicit correction, 2026-09-10

- User corrected the proposed plating-carrier policy: airline markup follows the first flight segment's `airlineCode` in the first requested route. This supersedes plating-carrier selection and prior mixed-carrier policy deferrals. First requested route and whole-journey passenger calculation remain as approved.
- [x] Shared Search/RePrice matching applies this policy to one-way, connecting, return and multicity fares. Later airlines and `platingCarrier` do not select a markup rule. Route/segment continuity and all-alternative validation remain.
- If first-route alternatives disagree on their first airline, or the first airline is missing/invalid, airline-specific matching remains unresolved rather than choosing an arbitrary alternative. Airline-independent rules still use a validated route. Existing audience priority and no-match fallback semantics are unchanged.
- Previous route-average proposals were not implemented. No supplier calls or release in this local increment. Evidence: [first-airline implementation](docs/evidence/FIRST_AIRLINE_MARKUP_2026-09-10.md).

### Branded fare availability — fresh UAT check, 2026-09-10

- [x] User requested confirmation. Three UAT-only direct Searches returned 35 one-way, 13 return and 16 multicity offers; all 64 have `brandedFares=null`. Supplier status arrays include partial failures, so this is not full-inventory or all-source acceptance.
- Supplied documentation has optional `brandedFares`/`brandedFareRefs` but no separate Brand Content endpoint. Actual populated brand content, account entitlement and selection schema remain unverified; do not classify the feature as universally unavailable.
- No Book/Issue, database writes, production supplier calls or release. Evidence: [fresh UAT brand check](docs/evidence/UAT_BRANDED_FARE_CHECK_2026-09-10.md).

### Optional brand content and component work deferred — 2026-09-10

- User instructed preserving supplier `brandedFares=null` exactly and planning brand content only when populated supplier data becomes available. Existing behavior preserves null; no content is synthesized. Brand implementation is deferred.
- [x] Authorized offline component review examined all 944 then-present evidence JSON files without parse failures. 579 files contain 133,923 `bookingComponents` occurrences, all arrays of length one. Repeated original/selling/captured copies are included; these are not unique offers.
- No multiple-component sample was found. Allocation work is deferred until real coverage evidence exists; current single-component pricing and rejection of unsupported multiple-component pricing remain. This audit does not settle historical tax discrepancies or prove that all supplier inventory is single-component.
- Existing synthetic unsupported-component regression passes. No new runtime change, supplier traffic, database writes or release. Evidence: [component review](docs/evidence/BOOKING_COMPONENT_COVERAGE_2026-09-10.md).

### Purchase flow order — user instruction, 2026-09-10

- User prioritizes Hold → retrieve held booking/PNR → Ticket Issue → Direct Issue. Before issue, retrieval is the booking/PNR record, not an already-issued ticket. Cancel remains deferred; all supplier mutations remain restricted to Triplover UAT.
- A fresh read of the previously successful UAT hold's original six references again returned `Record locator not found`, without a status/deadline. The later duplicate-booking attempt remains unresolved. No new Book or Issue was dispatched in this recheck; do not infer booking absence or retry automatically.
- Addressed the previously documented public Hold shape gap: missing Book flightInfo aggregate total/base/tax is accepted only with the existing matching passenger/component/itinerary checks. Missing fields stay absent; present mismatches/nulls remain unresolved. Accepted selling values are projected once into existing fields.
- Live successful public Hold → PNR acceptance and ticketing implementation/authorization/deadline verification remain open. Evidence: [hold continuation](docs/evidence/HOLD_CONTINUATION_2026-09-10.md).

### BS domestic Hold/PNR acceptance — 2026-09-10

- User authorized BS domestic one-way/return/multicity attempts and instructed not to use Robot/AI/System passenger names. Previously supplied private UAT passenger input was reused and checked.
- [x] One-way DAC→CGP: supplier Created + PNR; public 202 exposed a nullable-cabin comparison gap. Verified PNR and Admin recheck both return Booked. Fixed the comparison without changing flight/RBD/money checks or inventing cabin labels; existing one-way unknown state is preserved for reconciliation.
- [x] Return DAC→CXB→DAC after the fix: public Book 200/held, supplier Created, PNR and Admin recheck 200/Booked. Accepted selling fare stays unchanged; same-key replay sends no second Book. This closes a representative successful public UAT Hold→PNR acceptance case, not every supplier/carrier/lifecycle scenario.
- [ ] Two BS multicity Searches (JSR/SPD and CGP/RJH alternatives) returned supplier no-availability/no-eligible-fare errors. No multicity Book was sent. Broader multicity hold acceptance remains open.
- Exactly two supplier Hold calls; no Issue/Cancel/production traffic. 52 ordinary tests, full PostgreSQL suite and strict checks pass. Successful isolated UAT databases and private backups retained for continuation; failed-search/synthetic test databases removed. No commit/push/deploy.
- Evidence: [BS domestic validation](docs/evidence/BS_DOMESTIC_HOLD_2026-09-10.md). Ticket Issue and Direct Issue remain the next requested stages and are not completed by these Hold tests.

### User-selected BS five-route multicity recheck — 2026-09-10

- [x] Direct UAT Search for SPD→DAC (22 Sep), DAC→CGP (23 Sep), CGP→DAC (24 Sep), DAC→CXB (25 Sep), CXB→CGP (26 Sep), all 2026, BS preference and prior 1 ADT + 1 child mix.
- Supplier returned zero offers, isSuccess=false and no availability for the given criteria. No platform pricing rejection caused this result; a particular failing leg is not established. No RePrice/Book/Issue was sent.
- Evidence: [five-route Search](docs/evidence/BS_FIVE_ROUTE_SEARCH_2026-09-10.md). Multicity availability/hold acceptance remains open.

- Subsequent user-requested four-route recheck removed SPD→DAC and retained DAC→CGP 23 Sep, CGP→DAC 24 Sep, DAC→CXB 25 Sep, CXB→CGP 26 Sep 2026. Direct BS UAT Search again returned zero offers and supplier no availability. No Book/Issue; evidence appended to the same report.

- Three-route follow-up (DAC→CGP 23 Sep, CGP→DAC 24 Sep, DAC→CXB 25 Sep 2026) returned an explicit supplier limitation: `Current vendor cannot sell more than 2 OriginDestination. for USBangla`. Treat this as current BS UAT vendor-path evidence, not a global platform/supplier limit or proof of two-route availability. No automatic splitting into bookings, RePrice or Issue. Evidence appended to the same report.

- User-requested all-airline recheck of those three routes returned four BG bookable/refundable Search offers (supplier totals BDT 24,228 / 29,843 / 30,720 / 39,496 for 1 ADT + 1 child). BS vendor-limit and Sabre-token failures coexist with successful sources. No RePrice/Book/Issue; multicity Search availability is evidenced for BG, not yet its Hold acceptance. Same report contains evidence.

### BG multicity Hold — 2026-09-10

- [x] User authorized booking. Fresh BG Search → lowest eligible RePrice → acceptance → exactly one Hold succeeded for DAC→CGP 23 Sep, CGP→DAC 24 Sep, DAC→CXB 25 Sep 2026. Public Book 200/held, supplier Created + PNR. Same-key replay is identical without another dispatch.
- [x] PNR and Admin retrieval return 200, but status is Created and latest ticketing deadline empty. Multicity Hold/retrieval is evidenced; issue readiness and complete lifecycle acceptance remain open. No ticketing/direct issue/cancellation executed.
- Original supplier total BDT 24,228; accepted selling BDT 25,228 for the two previously supplied passengers. Independent exact pricing audit passes. Private evidence, isolated booking DB and backup retained; no production change or release. Evidence: [BG multicity Hold](docs/evidence/BG_MULTICITY_HOLD_2026-09-10.md).

### Public booking reference — 2026-09-11

- [x] User-selected `STR` + GDS PNR + airline PNR, e.g. `STR8FE94RKECOCE`, stored with the existing booking UUID. No numeric ST reference.
- [x] Owner-scoped `GET /api/bookings/by-reference/{reference}` retrieves the saved order. Book/replay/retrieve expose `X-Booking-Reference`; Admin shows Order Ref.
- [x] Migration backfill, immutable established reference, missing/multiple airline-PNR fallback to UUID, and duplicate-pair 409 handling. Reference possession does not bypass authentication.
- [x] Retained isolated BG UAT booking successfully retrieved by `STR8FE94RKECOCE` and UUID with identical saved responses, HTTP 200. No supplier calls, ticket issue or production migration in this step.
- Evidence: [public reference verification](docs/evidence/PUBLIC_BOOKING_REFERENCE_2026-09-11.md).

### Pending changes release authorization — 2026-09-11

- User explicitly authorized committing/pushing all pending work and updating the database. Include migrations 0013–0014 in the existing main-branch CI/CD backup → migrate → deploy workflow.
- Production Hold/Issue remains prohibited; this release authorization does not authorize supplier mutations or copying private UAT booking data into production. Deployment result will be recorded after verification.

### Pending changes release completed — 2026-09-11

- [x] All pending source, tests, docs, Admin assets and migrations 0013–0014 committed/pushed to main in `bdd3ec7606f27bac0c5dc47aca3a035b602569b5`.
- [x] GitHub Actions [run 34512844417](https://github.com/babuas25/shapontravels/actions/runs/34512844417): Rust/PostgreSQL checks, Linux release build and VPS deployment all succeeded. Existing activation workflow requires backup success before migrations and checks schema readiness after restart.
- [x] Independent SSH read-only checks: production `/health/live` ok, `/health/ready` ready, new `/api/bookings/by-reference/{reference}` present in served OpenAPI, `/admin/reconciliation` HTTP 200. Readiness verifies embedded migration checksums, including 0013–0014.
- No supplier mutation performed; no local UAT data, credentials or database dumps committed/copied to production. The prior local-only release notes are historical and superseded for this deployed code. Documentation-only follow-up uses `[skip ci]`; deployed application remains `bdd3ec7`.

### Held-ticket Issue authorization and implementation — 2026-09-11

- User explicitly authorized confirming holdable bookings in Triplover UAT and excluded Direct Issue. Production supplier Issue remains prohibited.
- Added `POST /api/ticket/NewTicket` and owner-scoped ticket retrieval, machine booking/ticketing permissions, database/transport enablement, exact Triplover UAT host gates and production deny. Search/RePrice `bookable=true`, verified held state, accepted fare and fresh PNR Booked/no-ticket/deadline checks are required.
- Durable unique per-booking issue reservation, key conflict protection, no transport retries, cancellation-resistant completion, immutable raw outcomes and redacted audit. Unknown outcomes remain blocked even with a different key. Original Hold replies stay unchanged; `X-Ticket-State` exposes the effective ticket state.
- Exactly one NewTicket sent for the retained BS return UAT Hold. Supplier succeeded with two ticket numbers; subsequent PNR returned Ticketed. No new Hold, Direct Issue, Cancel or production call was sent.
- First public result was 202 because Triplover returned a CNN child as CHD in ticket passenger metadata. All names, references, flight counts, itinerary and original fares matched. Added narrowly scoped Triplover child-label compatibility with unchanged flight-count/fare verification; saved-response verification returned HTTP 200 with two ticket passengers, and ticket retrieval matched exactly. No repeat Issue request was needed.
- Append-only `POST /api/bookings/{id}/ticket/verify` validates a captured success without supplier calls. Original unknown evidence remains immutable. Timeout/missing/invalid evidence cannot become issued through this endpoint.
- Migrations 0015–0016 applied to disposable tests and the privately backed-up retained BS return UAT database only. Local permissions/controls restored and temporary token removed. No commit, push or deployment in this increment.
- Production commercial authorization, Direct Issue, reports, missing-response ticket reconciliation and broader supplier/lifecycle acceptance remain unfinished. Detailed evidence: [held ticketing](docs/evidence/HELD_TICKETING_2026-09-11.md).

### Held-ticket release authorization — 2026-09-11

- User authorized database migrations and Git push for the completed held-ticket implementation. Release migrations 0015–0016 through the existing backup → migrate → deploy workflow, preserving production supplier execution restrictions.
- Production ticketing remains disabled by the application/transport gates. No supplier calls or private UAT data transfer are part of this release.

### Held-ticket release completed — 2026-09-11

- Committed and pushed held-ticket implementation to main as `08a569e`. GitHub Actions [run 34561334396](https://github.com/babuas25/shapontravels/actions/runs/34561334396) passed checks, Linux build and production deployment.
- Existing workflow completed backup → migrations 0015–0016 → grants → restart → readiness. Independent public HTTPS checks returned `/health/live` 200/ok and `/health/ready` 200/ready; served OpenAPI includes NewTicket, ticket retrieval and saved-ticket verification. Readiness verifies all embedded migration checksums.
- Local working database also backed up privately and migrated successfully; versions 15 and 16 confirmed successful. No UAT booking data transferred, supplier mutation or execution-gate change.
- Prior local-only notes describe the implementation stage and are superseded by this release. Documentation-only completion commit uses `[skip ci]`; deployed application remains `08a569e`.

### Ticket details/report continuation — 2026-09-11

- User requested continuation after held-ticket release; implemented the next proposed Ticket details/report API increment. Direct Issue, Cancel and production ticket execution remain excluded.
- Added owner-scoped report lookup by booking UUID, STR reference, and `GET /api/B2BReport/AirTicketingDetails/{platformSearchUuid}/Confirmed`. Ambiguous references/search transactions return 409; foreign/unknown records return 404; verified issued evidence and ticketing permission are required.
- Original supplier servicing uses a bounded GET with safely encoded stored transaction, read-only retries and servicing control independent of Search/ticket mutation flags.
- Reports validate PNR/transaction, ticket/passenger sets, original fare, passenger-type groups and itinerary when present. Public report preserves the top-level report shape and accepted selling prices without another markup; internal supplier account/reference/payment/financial extras are excluded.
- Migration 0017 stores immutable raw/report verification evidence and redacted audit without changing issue outcomes. Missing-success ticket reconciliation remains separate.
- Actual retained BS return UAT report returned Issued with two passengers, two flight segments and supplier total BDT 17,382; public report uses accepted selling BDT 18,382. No new booking, issue, cancellation or production supplier request was sent.
- This increment is local; production database, deployed binary and Git remote remain unchanged. Evidence and final checks: [ticket report](docs/evidence/TICKET_REPORT_2026-09-11.md).

### Ticket report release authorization — 2026-09-11

- User requested continuing the stated next step: migration 0017, Git push and deployment of the completed report increment through the established backup/migrate workflow. Production ticket mutation restrictions remain unchanged.

### Ticket report release completed — 2026-09-11

- Pushed report implementation to main as `cd3f2b5`. GitHub Actions [run 34562629848](https://github.com/babuas25/shapontravels/actions/runs/34562629848) passed Rust/PostgreSQL checks, release build and production deployment.
- Established backup → migration 0017 → grants → restart workflow succeeded. Independent HTTPS liveness/readiness checks returned 200/ok and 200/ready. Served OpenAPI includes all three report routes; unauthenticated report lookup returns 401.
- Local working database was privately backed up and migrated; migration 17 is recorded successful. No supplier mutation, production authenticated report read or private UAT data transfer occurred.
- Earlier local-only report notes are historical and superseded by this release. Documentation-only completion uses `[skip ci]`; deployed binary remains `cd3f2b5`.

### Uncertain Issue reconciliation continuation — 2026-09-11

- User requested the next stated step: Issue reconciliation after timeout/lost response. Added owner-scoped client reconciliation and human Admin unresolved queue/recheck APIs. No new supplier Issue/Book/Cancel is permitted by these operations.
- Require existing issue reservation, verified accepted Hold, original servicing/currency and references; pending waits five minutes, unknown can be read immediately. Ticketed PNR plus complete matching Issued report proves passenger/ticket/itinerary/original fare evidence. Not-found, Booked, incomplete or conflicting responses remain unresolved; reservation is never released for retry.
- Captured successful Issue/PNR ticket numbers must agree with report proof. Original pending/unknown evidence remains immutable; append-only verification and audit provide effective issued retrieval/replay. Concurrent client/Admin/saved-response verification serializes on the issue row.
- Added immutable reconciliation evidence in migration 0018. Admin returns safe summaries through APIs; no dedicated UI/background scheduler/manual bypass in this increment.
- Independently reconstructed the two existing UAT tickets from previously captured report evidence, without reading an Issue receipt as validator input, supplier calls or database writes. Changed fare/flight rejected. Actual supplier lost-response incident not manufactured; mock integration covers the timeout path.
- This increment is local and not pushed/deployed. Migration 0018 applied only to disposable test databases; no working/UAT/production migration or supplier mutation. Final evidence: [ticket reconciliation](docs/evidence/TICKET_RECONCILIATION_2026-09-11.md).

### Ticket reconciliation release authorization — 2026-09-11

- User explicitly requested deployment of migration 0018 and the completed reconciliation increment. Use the established backup → migrate → deploy workflow. Production supplier mutation restrictions remain unchanged.

### Ticket reconciliation release completed — 2026-09-11

- Committed/pushed reconciliation release as `15e8e58`. [Workflow 34564419172](https://github.com/babuas25/shapontravels/actions/runs/34564419172) checks, Ubuntu build and production deployment all succeeded.
- Established backup → migration 0018 → grants → restart workflow completed. Independent public HTTPS liveness/readiness returned 200/ok and 200/ready, confirming embedded migration checksums. All three reconciliation routes are served; unauthenticated client and Admin checks return 401.
- Local working database was privately backed up and migrated; migration 18 is successful. No supplier mutation or private UAT data transfer occurred. Production ticket execution restrictions remain unchanged.
- Earlier local-only notes describe implementation before this authorized release. Documentation-only completion uses `[skip ci]`; application release remains `15e8e58`.


### Direct Issue implementation authorization and local delivery — 2026-09-11

- User clarified that Direct Issue must be implemented, but real Issue testing is prohibited, and authorized proceeding. This supersedes earlier Direct Issue exclusions.
- Added explicit non-holdable intent validation, dual permissions and enablement checks, durable one-dispatch reservation, immutable execution mode, ticket receipt validation/storage and servicing compatibility. Subsequent NewTicket execution is rejected for Direct Issue bookings.
- Real adapters deny Direct Issue unconditionally. Only the test environment with an offline-capable transport may dispatch; no real supplier Book/Issue request was sent during this increment.
- Added migration 0019 and synthetic integration coverage for success, replay, duplicate offer, timeout, bad references/tickets/passengers/fares, live-environment denial, ticket/report retrieval and immutable mode. See [Direct Issue contract](docs/DIRECT_ISSUE.md).
- This increment remains local. Working/production migration, git push and deployment are pending.
- Final checks passed: formatting, Clippy with warnings denied, 60 regular tests and the complete disposable-database suite (43.90 seconds). All task-created temporary databases were removed; live/private opt-in tests were not run.

### Direct Issue release authorization — 2026-09-11

- User authorized continuing the proposed backup, migration 0019, git push and deployment sequence. The local working database was privately backed up, its archive checked, and migration 0019 applied successfully. Production release follows the established backup → migrate → deploy workflow. Real Direct Issue execution stays disabled; no supplier mutation test is authorized or performed.

### Direct Issue release completed — 2026-09-11

- Pushed application commit `0200efc`; [workflow 34566216193](https://github.com/babuas25/shapontravels/actions/runs/34566216193) passed Rust/PostgreSQL checks, Ubuntu release build and production deployment.
- Local working backup archive verified and migration 19 confirmed successful. Production workflow completed backup → migration 0019 → grants → restart → health checks. Independent public live/readiness returned 200/ok and 200/ready; OpenAPI serves the new Book receipt description.
- No real supplier Book/Issue test occurred. Real Direct Issue remains disabled. Earlier local-only statements describe the implementation before this authorized release; completion documentation is committed with `[skip ci]`.
