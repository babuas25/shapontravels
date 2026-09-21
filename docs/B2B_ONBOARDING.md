# B2B applicant flow

The Rust identity service and its PostgreSQL database own application accounts,
application decisions and agency membership. Clerk verifies sign-in; its role
metadata is not permission authority.

1. A verified first sign-in reads the Rust session. A missing `portal_users` row
   returns `onboarding_required` with no user. Reading the session does not write.
2. The frontend sends the browser to `/b2b-application`. Once that page mounts,
   it sends a same-origin `POST /api/identity/onboard` with an empty JSON object.
   The server derives the subject from the verified Clerk session and calls Rust
   `POST /admin/portal-identity/onboard` using the configured identity bridge.
3. Rust verifies the provider identity, checks pending invitations, account
   creation and retained-reference conflicts, then creates one `portal_users`
   row with role `customer` and status `onboarding`. Concurrent/repeated requests
   reuse the same row. Existing suspended/deleted accounts are not reactivated.
   The welcome outbox entry and audit are committed in the same transaction.
4. The applicant fills in business and personal details. `applications/submit`
   saves a versioned pending record in `portal_identity_applications`. Optional
   documents use the existing private document service. Submission does not grant
   B2B access. Pending applications display a review status and cannot be edited.
5. An authorized admin opens **Users & Roles → Review application**. Rejection
   records a note and permits the applicant to edit and resubmit. Approval uses
   Rust's existing atomic transaction to provision the agency/membership, activate
   the B2B role, transfer application details into `portal_identity_profiles`, and
   record the audit and provider/notification outboxes. The applicant cannot
   approve their own application.
6. **Check approval status**, a new sign-in or a visit to the application page
   reads current Rust authority. Approved B2B users go to
   `/dashboard/flight-search`. If an approval effect has revoked the old provider
   session, the user signs in again first. Suspended/deleted users stay blocked.

Provider and mail synchronization use existing workers. Tests use synthetic
providers and never deliver mail. Conflicting invitations or retained accounts
require the existing reviewed recovery process; do not bypass them with SQL role
updates. An uncertain setup request can be retried explicitly: Rust preserves the
existing account, and the frontend does not automatically replay failed writes.

## Verification

Backend regression:

```sh
# Set this only to a newly created, empty loopback database whose name ends
# in _identity_test. The test refuses nonempty databases and applies migrations.
IDENTITY_ONBOARDING_TEST_DATABASE_URL=postgres://USER@127.0.0.1:PORT/NEW_identity_test \
  cargo test --test identity_onboarding_flow -- --ignored --nocapture
```

Frontend repository:

```sh
npm run verify:b2b-onboarding
npm run verify:b2b-onboarding-ui
npm run verify:rust-identity-authority
npm run verify:rust-identity
npm run verify:identity-login-resilience
npm run typecheck
```

The Chrome fixture runs the actual React components against synthetic loopback
responses. Set `CHROME_PATH` if Chrome is not installed at the macOS default.
It covers initial setup, validation, pending review, rejection/resubmission,
approval navigation, explicit retry, expired sign-in and a mobile viewport.

## Local verification record — 2026-09-21

Implemented on both repositories' `development` workspaces. The backend needed no
runtime or schema change; the frontend connects the existing Rust onboarding API
and corrects entry/approval routing. The PostgreSQL journey passed against newly
created `onboarding_20260921_1_identity_test` on loopback port 55439, with synthetic
identities and all migrations applied. The frontend bridge, routing, resilience,
Chrome journey and TypeScript checks passed. Changes are not committed, pushed or
deployed. No live database, service, identity pin or notification worker changed.


## Company profile completion — 2026-09-21 (local, not deployed)

B2B owners and sub users may fill blank text fields in their own Company profile
once. Each successful save locks that field. Existing values, including details
transferred from an approved application, are already locked. Later corrections
or removals require a call to Admin or Super Admin, who edits the profile through
Users & Roles. The restriction covers personal, business and bank text fields;
company documents and logos follow the owner permissions below.

Rust enforces the rule against current PostgreSQL values inside the existing
versioned, audited transaction. Forged overwrite/clear attempts return
`IDENTITY_PROFILE_FIELD_LOCKED`. Failed mixed patches roll back. Concurrent first
saves cannot overwrite each other, and an exact operation replay remains safe.
No new tables or migration are needed.

Verification passed in new local PostgreSQL databases
`profile_fill_once_20260921_1_identity_test` and
`profile_matrix_20260921_1_identity_test`: first saves, normalization, locked-field
replacement/clear rejection, mixed-patch atomicity, concurrency, operation replay,
admin/superadmin corrections, account isolation, suspension and document scope.
Chrome verified blank fields, immediate/persistent locks, admin corrections and
call-admin guidance. Profile/Phase 5 adapter checks and TypeScript passed.
Deploy the tested backend before the frontend so the new blank-field controls
have matching server permission support. No service, live database, authority
pin, commit, push or deployment was changed in this task.


## Company documents — 2026-09-21 (local, not deployed)

An active B2B agency owner can update their own company logo anytime. Trade
licenses allow one successful upload per July–June year, resetting at 00:00 on
1 July in Bangladesh. CAAB licenses allow another upload two calendar years
from the last successful upload date and time. For example, an upload on
21 September 2026 becomes eligible on 21 September 2028. Each successful update
starts another two-year wait; a 29 February anniversary clamps to 28 February.
The latest request replaces the earlier proposed January CAAB cycle.

The first upload uses the allowance. Failed or unconfirmed uploads do not use
it until successful publication. Rust checks permissions and renewal dates at
prepare, start and finish, using the internal PostgreSQL asset history and
server clock. Concurrent publication and replay cannot bypass the limits.
Removing a document does not erase its history. Admin replacements also become
the latest successful upload. No schema migration is required.

Owners may remove their logo. License removal, TIN/NID edits and company document
changes by sub users remain admin-managed. Admin/Super Admin can correct any
company document when contacted. Text fields retain the fill-once rule above.
The UI labels the existing `travelAgencyLicense` slot **CAAB License**, hides
unavailable upload controls, and shows the next allowed date in Bangladesh time.

A separate `documents/policy` endpoint preserves the existing strict document
response contract. Deploy backend support before the updated frontend.
Verification passed: calendar boundaries/leap days, PostgreSQL renewal/history/
concurrency/permissions (`document_policy_20260921_1_identity_test`), the full
application/document/name matrix (`phase5_documents_20260921_3_identity_test`),
Chrome company-profile journeys, Phase 5 adapter checks and TypeScript.
The full matrix used a fresh UTF-8 database for Bangla-field coverage and the
current recipient-only activation-mail expectation. All uploads/providers were
synthetic. No live database change, push or deployment was performed.


## Pending application after a separate agency activation

An older account-management flow allowed an administrator to provision the agency
while its application remained pending. Such an active B2B owner can now have its
application approved using its existing active agency and linked wallet. Approval
still checks the application, identity and profile versions and administrator
scope, transfers only missing profile fields, retains review history and queues
one registration confirmation. It does not create a second agency or wallet.
Rejecting an already active partner through application review is disallowed;
access suspension is a separate account-management decision.

New manual agency provisioning rejects a pending application with
`IDENTITY_APPLICATION_REVIEW_REQUIRED`; the administrator must review it first.
The pending queue includes retained active owners awaiting application approval.
Real optimistic version conflicts refresh the frontend's application snapshot
without automatically repeating the administrator's decision.
