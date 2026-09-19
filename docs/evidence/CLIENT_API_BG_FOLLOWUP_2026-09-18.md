# Client API — independent BG UAT follow-up

পরবর্তী ফলাফল: [BS successful journey ও BG Issue investigation](CLIENT_API_ISSUE_FOLLOWUP_2026-09-18.md)। নিচের report আগের BG attempt-এর historical snapshot।

**রায়: repeated `202 outcome_unknown`-এর supplier-side দ্বিতীয় failure class পাওয়া গেছে। Client rollout blocker বহাল; নতুন successful Book→Issue এখনও যাচাই হয়নি।**

18 September 2026-এ নতুন isolated database ও নতুন synthetic traveller দিয়ে BG-এর **DAC→CGP, 23 October 2026** flight পরীক্ষা করা হয়েছে। Departure 35 দিন পরে, requested minimum 15 দিনের মধ্যে কোনো flight পরীক্ষা হয়নি। আগের unresolved booking বদলানো বা আবার dispatch করা হয়নি। এটি [remediation report](CLIENT_API_REMEDIATION_2026-09-18.md)-এর অতিরিক্ত evidence।

## Live ফলাফল

| ধাপ | ফলাফল |
| --- | --- |
| Search | HTTP 200; 6 offers |
| FareRules / Reprice / acceptance / pricing | প্রত্যেকটি HTTP 200 |
| Book | HTTP 202, `outcome_unknown`, **3.357 seconds** |
| Safe diagnostic | `SUPPLIER_REPORTED_FAILURE`, `nextAction=contact_support`, `automaticRetryAllowed=false` |
| Same-key replay | Exact same status/body; additional supplier dispatch 0 |
| Supplier calls | Book 1; Issue 0 |
| Wallet ledger | Entries 0 |

Booking ID: `c133cf8e-3796-4c05-9f89-fd0426722f11`। Saved response-এ supplier `isSuccess=false` ও Amadeus Book-এ invalid data বলেছে; কোন field invalid বলা হয়নি। এটি আগের 60-second timeout থেকে পৃথক। Raw internal service address client response ও এই report-এ প্রকাশ করা হয়নি।

নিজের saved transaction দিয়ে read-only supplier GlobalSearchB2B ও AirTicketingDetails lookup দুটিই HTTP 200। Global result ও report-এ status `Failed`; report-এ `statusFor=Booking`, `isCompleted=true`, একই Amadeus invalid-data reason। PNR/ticket number অনুপস্থিত। এটি supplier report-এর অবস্থা; airline reservation না হওয়ার চূড়ান্ত নিশ্চয়তা হিসেবে ব্যবহার করে local booking close করা হয়নি।

## Payload পরীক্ষা ও সীমা

- Outgoing Book-এর `uniqueTransID`, `itemCodeRef`, `priceCodeRef` actual supplier Reprice response-এর সঙ্গে হুবহু মিলে।
- Supplier-এর [published UAT OpenAPI](https://userapi-uat.triplover.com/swagger/v1/swagger.json)-এর `AirBookRequest` ও nested models অনুযায়ী unknown property, JSON type, required property বা declared `maxLength` violation পাওয়া যায়নি। এই structural check supplier business validation সম্পূর্ণ যাচাই করে না।
- 10 September-এর successful BG multicity evidence-এর সঙ্গে shape comparison-এ current request-এ `isLeadPassenger` ও `documentType` নেই; successful request-এ এগুলি ছিল (document type ছিল empty string)। Published schema-তে এগুলি required নয়। বর্তমান failure-এর কারণ হিসেবে কোনোটি নিশ্চিত হয়নি। Historical success-এর itinerary ও passenger count-ও আলাদা; এটি controlled A/B comparison নয়।
- Supplier schema `dateOfBirth`-কে `date-time` বলে; current ও historical successful request দুটিই date-only পাঠিয়েছে। Format validation এই structural check-এর অন্তর্ভুক্ত নয়। Supplier-এর runtime expectations স্পষ্ট করা প্রয়োজন; এই evidence থেকে date format-কে root cause বলা যায় না।
- একই Amadeus invalid-data failure [16 September-এর portal investigation](PORTAL_BOOK_FAILURE_2026-09-16.md)-এও recorded। ফলে এটি শুধু নতুন diagnostic code যোগ করার পরে দেখা গেছে এমন নয়।

কোনো অনুমানভিত্তিক payload পরিবর্তন বা নতুন booking দিয়ে unresolved intent retry করা হয়নি। Supplier-এর field-level rejection detail এবং reservation outcome confirmation প্রয়োজন। Private incident note প্রস্তুত; supplier-কে পাঠানো হয়নি।

## Verification ও retained evidence

- Opt-in BG UAT audit: 1 test passed; journey profile প্রথম dispatch-এর পরে alternate route selection বন্ধ করে। Passing test এখানে safety assertions পূরণ বোঝায়; successful booking বোঝায় না।
- নতুন **8 public responses / 8 fare-breakdown snapshots** served client contract ও amount equations দিয়ে validate হয়েছে; failures 0। আগের 54/396 evidence count-এর বাইরে এই অতিরিক্ত count।
- Targeted audit Clippy with warnings denied, formatting ও diff whitespace checks passed। এই ধাপে application source পরিবর্তন হয়নি; UAT harness-এ BG alternate profile, minimum date guard, private request capture ও exact replay assertion যোগ হয়েছে। পূর্বের full regression results মূল remediation report-এ আছে।
- Private captures ও database archive `.local/client-api-audit-20260918/bg-followup-evidence/`-এ retained। Archive listing verified; restore test করা হয়নি। এই run-এর জন্য চালু করা isolated test cluster কাজ শেষে বন্ধ করা হয়েছে। Working/production database বা deployment পরিবর্তন হয়নি।

Machine-readable evidence: [verification JSON](CLIENT_API_BG_FOLLOWUP_VERIFICATION_2026-09-18.json)।

## Client handoff decision

Client-কে `contact_support`-এর পরে বারবার polling বা নতুন idempotency key দিয়ে automatic retry করতে বলা যাবে না। বর্তমান diagnostics এই decision স্পষ্ট করে; এটি supplier booking reliability সারায় না। Supplier কারণ নিশ্চিত করে successful independent Book→Issue ও single wallet debit verification সম্পন্ন হলে rollout decision পুনর্বিবেচনা করা যাবে।
