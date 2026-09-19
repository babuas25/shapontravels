# Import completion and wallet availability

Airline, Supplier API and Manual forms now hide import/authorization actions after successful completion. The booking link remains available. Changing booking inputs resets the form for another import; controls are disabled while an import/retrieval is in progress.

The selected owner resolves to the canonical agency wallet in Rust. A Super Admin-only, uncached wallet read returns available/hold minor units and frozen/active status. The frontend shows available balance next to owner selection, compares the current payable with exact integer arithmetic, and shows the shortage. Previous-owner/currency responses cannot enable a charge. Failed/loading/missing/frozen wallet reads block confirmed import. On Hold imports retain their no-charge workflow. Confirmed supplier imports no longer offer a historical no-wallet button.

Rust authorization checks current available balance for new confirmed imports, while the existing atomic reserve/capture remains the final authority against concurrent spending. Exact already-captured imports retain authorization recovery semantics; duplicate import requests do not create a second debit.

Verification:

- `tsc --noEmit` and targeted ESLint passed for the import forms, shared wallet component, bridge route and identity request policy.
- `cargo clippy --locked --lib --test dashboard_sections -- -D warnings` and local runtime build passed.
- Disposable database integration passed: canonical owner/sub-user balances, missing currency account, denied non-Super-Admin read, unsupported currency, insufficient authorization/import and rollback, confirmed no-charge rejection, exact import recovery and existing API retrieval tests.
- Authenticated localhost UI showed the same selected agency balance on all three forms. Manual confirmed import enabled at exactly available balance and disabled with a 0.01 shortfall, showing that exact shortage. Currency changes immediately blocked charge; On Hold remained available without a debit.
- Browser verification did not submit any import or wallet charge. Temporary test inputs/tab were discarded. Local backend was restarted with the updated code; no migration or production deployment was required.
