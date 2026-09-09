# Triplover UAT login check — 2026-09-08

User authorized continuing with saved UAT credentials. The diagnostic asserts the UAT Search/User host names and supplied test account before constructing the adapter. No production references are used.

Adapter authentication failed before Search dispatch. One additional Login-only diagnostic returned HTTP 200 with isSuccess=false, message="Invalid Format", and no data/token. Thus this is a supplier business rejection, not a successful Login or an observed token-expiry parsing failure.

The supplied API documentation requires the supplier-issued base64-encoded password to be passed unchanged (Triploaver_API_Documentation.md:107). The configured password fails a local standard-base64 syntax check. This supports a credential-format problem but does not prove the supplier's exact validation rule or that automatic encoding would fix it. No password was encoded, substituted, printed or written to the report.

Two Login requests total; no Search, Reprice, Book, PNR, Cancel or Issue request was sent. Active settings were not changed. Evidence is under `.local/evidence/uat-tk-cnn-20260908` with sanitized login outcome.
