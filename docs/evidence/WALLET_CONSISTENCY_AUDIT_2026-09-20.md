# Wallet consistency audit — 20 September 2026

**Follow-up:** F1 ও F2-এর code fix এবং linked deposit reversal implementation সম্পন্ন। নিচের audit findings মূল পরীক্ষার সময়ের অবস্থা; সংশোধনের আচরণ ও rollout নির্দেশনা [Wallet deposit controls](../WALLET_DEPOSIT_CONTROLS.md)-এ আছে। Production deployment বা বাস্তব financial correction করা হয়নি।

পরীক্ষায় **দুটি নিশ্চিত সমস্যা** পাওয়া গেছে: একই বাস্তব পেমেন্টের একাধিক deposit request অনুমোদন করা যায় এবং booking payment report-এ post-ticket লেনদেনের সম্পূর্ণ হিসাব আসে না। পরীক্ষিত ledger arithmetic ও reservation invariants ঠিক ছিল।

## Scope and safety

- Rust checkout: `6530399`; পাশাপাশি `shopontravels`-এর বর্তমান Rust wallet ও ticket-management adapters পড়া এবং hermetic adapter tests চালানো হয়েছে। ওই checkout-এর আগে থেকে থাকা পরিবর্তন স্পর্শ করা হয়নি।
- কোনো বাস্তব টিকেট ইস্যু, supplier Book/NewTicket/Cancel, বাস্তব refund/reissue/void, deposit, email বা SMS করা হয়নি।
- নতুন isolated PostgreSQL cluster, `127.0.0.1:56439`, এবং কৃত্রিম ডেটা ব্যবহার করা হয়েছে। Issue tests কেবল mock supplier ব্যবহার করেছে। শেষে ওই cluster বন্ধ করা হয়েছে।
- চলমান production database-এর balance audit করা হয়নি: runtime `DATABASE_URL` ছিল না, configured local API/মূল PostgreSQL চলছিল না। নিচের findings বর্তমান code ও synthetic reproduction-এর; এগুলো production-এ ইতিমধ্যে টাকা ভুল জমা হয়েছে—এমন দাবি নয়।
- অ্যাপ্লিকেশনের লজিক বা migration পরিবর্তন করা হয়নি।

## F1 — High: একই external payment আলাদা request ID দিয়ে দুবার credit হয়

**Reproduction:** একই MFS destination, transaction ID `AUDIT-ONE-PAYMENT-1`, date, amount ও receipt SHA-256 দিয়ে দুটি আলাদা UUID-তে deposit জমা দেওয়া হয়েছে। একই অনুমোদিত finance reviewer দুইটি request approve করেছেন। প্রতিটি submit ও approval HTTP 200 দিয়েছে।

| হিসাব | BDT |
| --- | ---: |
| একটি পেমেন্টের gross | 100.00 |
| fixture-এর 5% fee বাদে প্রাপ্য net | 95.00 |
| দুই request approve করার পর available balance বৃদ্ধি | **190.00** |

এটি approval ছাড়া credit হওয়ার সমস্যা নয়। একই request ID-তে retry/concurrent approval সঠিকভাবে একবার credit করে। কিন্তু একই পেমেন্ট আবার নতুন request ID-তে দিলে আগের credit-এর সঙ্গে মিলিয়ে আটকায় না।

**কারণ:** `src/wallet/workflows.rs:100`-এর replay lookup শুধু request UUID দেখে; `:187`-এর lock-ও UUID অনুযায়ী। `:301`-এ নতুন request insert হয়। Approval posting key `request:{id}`। `migrations/0030_wallet_workflows.sql`-এ external payment identity-এর uniqueness নেই। Receipt digest request hash-এ থাকলেও একাধিক request-এর মধ্যে duplicate check হয় না।

**প্রস্তাবিত সংশোধন:** payment method অনুযায়ী external payment identity নির্ধারণ করে approval-এর সময় atomically একবার claim করতে হবে; বিশেষ করে MFS destination/provider ও transaction ID দিয়ে duplicate approved credit আটকাতে হবে। Rejected request পুনঃজমা এবং bank reference-এর সম্ভাব্য পুনর্ব্যবহারের policy আলাদা করে নির্ধারণ করা দরকার। নতুন UUID, concurrent approvals এবং অন্য owner থেকে একই payment reuse-এর regression coverage দরকার।

Evidence: `.local/wallet-audit-20260920/duplicate-payment-test.log`; reproducer source `duplicate_payment_probe.rs` একই directory-তে। Temporary integration-test file সরানো হয়েছে।

## F2 — Medium: booking payment report রিইস্যু/ভয়েডের অতিরিক্ত লেনদেন বাদ দেয়

`src/wallet/booking_payments.sql:1` শুধু `subject_kind='ticket_issue'` operation থেকে captured/refunded amount ও payment state নেয়। Reissue/fee-dominant void-এর capture হয় `ticket_management` operation-এ। Reissue fare difference ফেরত দিলে সেই refund-ও নতুন operation-এ থাকে। ফলে ledger ঠিক হলেও report কম amount দেখায়।

মূল report SQL সরাসরি চালিয়ে synthetic booking-এর সম্পূর্ণ captured operations-এর সঙ্গে তুলনা:

| Fixture | রিপোর্টে capture | প্রকৃত capture | রিপোর্টে refund | প্রকৃত refund |
| --- | ---: | ---: | ---: | ---: |
| `STRTEST01AIR001` — reissue ও পরবর্তী refund | 150.03 | **166.03** | 140.02 | **152.36** |
| `STRVOID03AIR001` — অতিরিক্ত void fee debit | 150.03 | **170.02** | 0.00 | 0.00 |

সব amount BDT। প্রথম booking-এ report-এর net charge 10.01, অথচ ledger অনুযায়ী 13.67। Refund থাকা সত্ত্বেও report-এর payment state `captured` থাকে।

Portal-এর `lib/wallet/rust-wire.ts` এই fields সরাসরি রূপান্তর করে এবং `components/dashboard/wallet/WalletReportDashboard.tsx` প্রদর্শন করে। তাই এটি ব্যবহারকারীর রিপোর্টেও পৌঁছায়; শুধু অভ্যন্তরীণ query-এর সমস্যা নয়।

**প্রস্তাবিত সংশোধন:** owner/account/currency ও booking অনুযায়ী সব relevant operation-এর capture, hold, release ও refund aggregate করতে হবে। Original payable এবং পরবর্তী charge আলাদা field-এ রাখা যেতে পারে; lifecycle payment state ticket-management outcome-এর সঙ্গে মিলতে হবে। একটি reissue → refund এবং fee-dominant void fixture দিয়ে report বনাম ledger equality যাচাই করতে হবে।

Evidence: `.local/wallet-audit-20260920/report-comparison.sql` এবং `report-comparison.txt`।

## Reverse-এর অর্থ অনুযায়ী ফলাফল

- **Issue hold release:** supplier non-issuance evidence ও আলাদা reviewer-এর অনুমোদন লাগে। Concurrent release/capture, replay এবং পরে positive ticket evidence আসার tests পাস করেছে। Timeout নিজে টাকা release করে না।
- **Reissue/void hold release:** approved debit hold release করে নতুন quote-এর জন্য request পুনরায় খোলা যায়। Reissue release/requote test পাস করেছে।
- **Approved deposit reversal:** বর্তমান Rust wallet-এ original ledger entry-র সঙ্গে বাঁধা dedicated reversal command নেই। Generic debit/credit adjustment আছে; এতে original deposit ID, remaining reversible amount বা একই deposit দুবার reverse ঠেকানোর invariant নেই। এটি feature/control gap; এই audit-এ actual duplicate reversal ঘটেছে বলে দাবি করা হচ্ছে না। ইতিমধ্যে approved deposit-কে rejected করে credit মুছে দেওয়া যায় না।
- **Imported/manual ticket:** import charge একই wallet kernel ব্যবহার করে, কিন্তু native ticket-management booking lookup imported records গ্রহণ করে না (`src/wallet/ticket_management/store.rs:89`)। তাই native refund/reissue/void পরীক্ষার ফল imported/manual ticket-এ প্রযোজ্য ধরে নেওয়া যাবে না।

## Verification

| Check | Result |
| --- | --- |
| `cargo test --locked --lib` | 95 passed; 2 opt-in tests ignored |
| `cargo test --locked --test wallet -- --ignored --nocapture` | Passed: exact money, overspend concurrency, replay, freeze, refund cap, maker/checker, fee snapshot, approvals, reporting aggregates, runtime privileges |
| Ticket-management `database_journeys_and_financial_invariants` | Passed: refund, reissue → refund, hold release/requote, void credit/zero/debit, expiry, insufficient funds, immutable evidence, client ownership/replay |
| `cargo test --locked --test database -- --ignored --nocapture` | Passed: migrations and mock-backed booking/ticket/non-issuance/recovery/lock-order regressions |
| `node scripts/verify-rust-wallet.mjs` in portal checkout | Passed; external network forbidden by harness |
| `node scripts/verify-rust-ticket-management.mjs` in portal checkout | Passed; mocked backend, zero legacy database/email calls |
| New duplicate-payment probe | Confirmed F1: two credits for the same external payment after two approvals |
| Actual booking report SQL vs operations | Confirmed F2 |

Read-only invariant queries over the two focused test databases covered 4 accounts, 13 operations and 38 ledger entries. **Zero mismatches** in account vs latest ledger, ledger arithmetic/sequence continuity, held balance vs open reservations, operation settlement postings, cumulative refunds, and approved request amount/account/currency vs its ledger entry. F1 still passes these arithmetic checks because both duplicate credits are individually valid ledger postings.

All logs and the reusable read-only SQL are under `.local/wallet-audit-20260920/`. Production balances, browser journeys against the running deployment, and live supplier outcomes remain unverified.

## Remediation verification

- F1: migration 0059 adds a durable unique external-payment claim at approval. New request IDs, different owners, changed amount/date/proof and concurrent approvals cannot claim the same mobile payment twice. Rejected submissions can be resubmitted; ordinary same-request retries remain successful. Failed approvals roll back the ledger posting and account version.
- F2: report captures/refunds aggregate the original issue and subsequent ticket-management operations, scoped to the same booking/account/currency. Reissue → refund and all three void directions match exact expected totals. Original payable remains unchanged. Held/released amounts are separate.
- Reversal: original approved deposit, net amount cap, immutable source binding, cumulative partial reversals, competing approvals, maker/checker separation, replay, frozen wallet and insufficient-funds rollback all pass. A reversed deposit's external payment remains claimed.
- Upgrade: a synthetic schema-0058 database with two already-approved duplicate deposits upgrades successfully; both historical credits remain intact and a third approval is blocked. Migration replay is safe.
- `cargo test --locked`: **125 passed, 0 failed, 45 opt-in tests ignored**. The focused wallet, upgrade, ticket-management and mock-backed database suites also passed against isolated local databases.
- Rust clippy with warnings denied, formatting, portal TypeScript checks and changed-file ESLint passed. Four portal runners passed: wallet controls, deposit-loading/reversal UI, existing wallet adapter and existing ticket-management adapter.
- One overly broad `--lib -- --ignored` invocation also selected the unrelated private UAT recovery test; it stopped at its missing explicit-opt-in assertion without reading private evidence or making supplier calls. That log is retained as `final-database-tests.log`; subsequent ordinary and explicitly selected integration runs passed. The ticket-management test in that invocation passed.
- Main evidence: `release-controls-tests.log`, `final-integrations.log`, `final-ordinary-tests.log`, `release-clippy.log`, and the portal verification outputs. All financial writes were confined to disposable synthetic databases.

No historical duplicate credits or prior unlinked manual adjustments were automatically reversed. Deployment requires migration 0059, the updated Rust service and the matching portal changes. The implementation is complete locally; the running production system has not been changed.
