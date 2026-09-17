# Saved passengers — local integration

Migration `0026_passenger_profiles.sql` moves the existing saved-passenger store to Rust PostgreSQL. It keeps the Supabase `0103_passenger_profiles.sql` field names, immutable `STPYYMMDD######` references, daily Dhaka reference counters and original Clerk owner IDs. Saved profiles remain separate from booking snapshots. This increment does not migrate booking, wallet, supplier execution or account lifecycle deletion.

The existing frontend `/api/passengers` contract now calls the authenticated Rust bridge. `PassengerManager` and the saved-profile selector in `BookingCheckout` retain their current design and selection logic. New Rust booking checkout remains a separate step. No direct Supabase passenger adapter remains in the frontend.

## Access and behavior

- The frontend reads the current active Clerk account and role for each request. A development role-preview cookie cannot authorize access.
- Rust requires the trusted frontend's Super Admin session for `/admin/portal-passengers`; commercial API and prebooking tokens are rejected. The trusted server supplies the Clerk actor, never the browser.
- Admin/Super Admin list and edit all profiles and may delete. B2B owners and sub-users list/edit their own Clerk-owned profiles independently. Customer access follows the existing B2C enablement flag. Other staff are denied.
- GET preserves the existing limit of 100 profiles, newest first, with optional passenger-type filtering. Existing frontend search filters the loaded profiles. Child/infant aliases remain compatible in the saved-passenger selector.
- Required identity fields, title/type/gender matching, optional contacts/documents/loyalty/SSR, dashboard age validation and checkout save semantics remain in place. Selecting a profile does not save it automatically. PATCH writes only submitted fields, preserving untouched imported values.
- Writes share the existing 40/hour actor budget; reads have a 120/minute budget. Ownership and public references are immutable. Audits contain identifiers, not passenger contents.

## Local copy completed

On 2026-09-14, the previously saved local database dumps were checked and contained no `passenger_profiles` table. The existing frontend connection backup was then used for authorized read-only Supabase GET requests. A private local JSON export was created and imported into `api_portal_local_clerk_20260914` on `127.0.0.1:55439`.

All **103 records belonging to 16 owners** were inserted and verified against every original column, including IDs, owners, references and timestamp instants. No owners were reassigned. The local export and pre-import database backup are under ignored `.local/passenger-import/`, with files restricted to the local OS user. Export SHA-256: `2a46820ffc739fad44daacf455b1bd700e367358b1814c476b4e7fa7bbb14d19`.

The import uses `../shopontravels/scripts/import-passengers-local.py`. It accepts a complete CSV or JSON passenger export, rejects conflicting existing rows, imports transactionally, advances reference counters and is idempotent. It refuses targets outside `api_portal_local_*` on `127.0.0.1:55439`. Only counts/checksums are logged. The source database was not modified.

## Verification and local runtime

The PostgreSQL integration suite covers owner isolation, sub-users, Admin access, token rejection, partial edits, immutable ownership, validation, rate limits, reference concurrency, audit privacy and no booking changes. The frontend `verify:rust-passengers` test exercises real Next handlers → Rust HTTP → disposable PostgreSQL with synthetic Clerk identities. Import checks cover CSV/JSON, exact values/timestamps, repeat imports, conflict rollback, atomicity and counter continuity.

The existing passenger page is available at `http://localhost:3000/dashboard/co-travelers`, using real Clerk development authentication and Rust on port 18081. Its local development bridge configuration is ignored and private. The UI keeps its previous 100-record display limit. The older port-13002 flight fixture is not the passenger application.

No commit, push or deployment was made for this increment.
