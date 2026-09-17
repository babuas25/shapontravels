# Local search credential reload — 17 September 2026

The active local portal was using the Triplover UAT-only example and cached credentials loaded before the supplier `.env` was changed to production. FirstTrip and TakeOff search participation was disabled in the selected local database. The original FirstTrip email-key typo was already corrected in the file by the time implementation began.

The canonical `local_identity` example now loads FirstTrip, TakeOff and Triplover from the supplier settings in Rust `.env`, without a hardcoded UAT host restriction. Credentials and URLs must still match the supplier account/environment. The local wrapper permits Search, FareRules and RePrice only; all booking, ticketing, cancellation and PNR/report methods deny access, regardless of write-enabled environment flags.

`node scripts/portal-dev.mjs` delegates to the canonical launcher, which watches `.env`, gracefully restarts the backend on changes and clears supplier authentication sessions. On restart, configured suppliers are enabled for search and previous offer availability epochs are invalidated. Missing email and password omit a supplier; partial configuration fails startup with a credential-safe message. Database identity configuration is never imported from the supplier file. Existing supplier failure isolation is preserved, with safe per-supplier failure logs added.

Validation:

- Signed-in browser Search: DAC → JSR, 30 September 2026, one adult, Economy. The previously unavailable page now renders two US-Bangla flights through TakeOff, from BDT 5,526.65.
- Saving a harmless `.env` comment automatically changed the backend process; readiness returned HTTP 200. The same browser search succeeded again after reload.
- Latest backend search returned three retained offers from seven source offers in approximately 2.2 seconds. The frontend displays two schedule choices. Previous offers have old epochs; new offers have current epochs.
- Local adapter tests: 2 passed, including UAT/production configuration and denial of every write method despite enabled flags. Supplier transport tests: 10 passed. Foundation tests: 6 passed. Strict Clippy for `local_identity`, JavaScript syntax check and diff whitespace check passed.
- Local persisted booking count remained 5; ticket issue count remained 0. No live Book, Issue, Cancel or PNR/report operation was submitted. No production deployment or database migration was performed.

Private before/after metadata is under `.local/evidence/search-credential-reload-20260917/`. Restart guidance: [LOCAL_PORTAL.md](../LOCAL_PORTAL.md).
