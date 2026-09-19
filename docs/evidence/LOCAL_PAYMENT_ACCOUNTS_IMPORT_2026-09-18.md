# Local payment account data import — 18 September 2026

The user requested copying admin payment account details from `/Users/ashifbabu/Documents/shapontravels-frontend` into the current application, without connecting the applications' databases. After confirming that the folder contained code rather than exported account records, the user explicitly authorized a one-time read-only source export.

- Source reads: GET-only exports of `wallet_company_bank_accounts` and `wallet_company_mfs_accounts`. No source writes or unrelated data export.
- Destination: existing loopback PostgreSQL database `portal_local_20260917_identity_test`, `wallet_settings` table.
- Imported: 7 active bank accounts and 3 active MFS accounts. Existing payment account rows were absent; import aborts if any appear before the transaction commits.
- Retained: source IDs, account and branch details, routing/SWIFT codes, active flags, source timestamps, sort-order metadata, MFS payment types/fees and logo/QR asset metadata.
- Existing image storage matches the source image storage. No image-storage configuration or credentials were changed.
- Import validation: field lengths/types, UUIDs, asset metadata, payment types and charge ranges; transaction verifies exact row data/active flags/kinds/timestamps and records one audit event per account.
- A full local database backup was made before import. Wallet account, request and ledger fingerprints were identical before and after import.
- Browser verification: the signed-in Super Admin Bank Accounts tab shows all 7 active banks with account/branch/routing/SWIFT details. The MFS tab shows all 3 active entries: bKash Send Money (1.10%), bKash Merchant (1.20%), and Nagad Merchant (1.50%), with retained logo/QR controls.
- No application database connection, environment setting, synchronization task or source credentials were added to the running application.

Private export, mapped data, backup and verification are retained in the Git-ignored `.local/bank-import-20260918/` directory. Export SHA-256: `28e7da59584b409f05939d93cae185c06421036efcb09d116f72c152cf24195e`.
