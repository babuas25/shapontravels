# Flight Aggregation & Booking API — Requirements

Status: Implementation in progress — Steps 1–2 verified complete; Step 5 implemented within documented coverage; Steps 3–4 and 6 partially implemented; Step 7 pending; Step 8 partially verified. Latest prebooking/release verification update: 2026-09-10. Historical milestone notes below are superseded where later updates say so.

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
- নিজস্ব frontend বা full administrative dashboard নির্মাণ; protected admin API scope-এ আছে।
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
