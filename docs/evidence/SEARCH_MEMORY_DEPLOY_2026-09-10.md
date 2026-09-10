# Persistence and response memory release — 2026-09-10

User explicitly authorized completing Git push and remaining database/deployment work, superseding the earlier release deferral.

- Application commit `b08b34d8dbc223f3c37fb75d64db3f813f19ff6d` pushed to main.
- [GitHub Actions run 34482728952](https://github.com/babuas25/shapontravels/actions/runs/34482728952): Rust/PostgreSQL checks, Ubuntu release build and VPS deployment all succeeded. Local release-script verification also passed all five tests.
- Activation log confirms this exact commit at 13:34:45 UTC / 19:34:45 Asia/Dhaka. The configured activation sequence completed backup, migration validation/execution, runtime grants and live/ready checks. There are no new migration files in this release; no local database contents were uploaded. Migration command reported success at 13:34:43 UTC, and subsequent readiness checks validate the expected migration checksums.
- Eight independent post-activation HTTPS checks passed: live/ready 200, unauthenticated Search/admin 401, and four OpenAPI encoding negotiation cases with byte-identical decoded content. Request IDs and no-store remained present. These checks do not invoke supplier Search or exercise authenticated production fare processing.
- Read-only production observer confirms active service with new PID 75311, cleanup startup at 15-minute retention / 30-second interval, and zero eligible cleanup backlog, expired markers, RePrice or booking rows at observation.

Release includes 16-offer INSERT batches, aggregate persistence timing, and consuming Search response serialization. Existing four-active/eight-queued/two-second admission defaults remain unchanged. No production environment/Nginx change was needed. Local memory improvements and their timing/methodology limits remain in the [persistence](SEARCH_PERSISTENCE_2026-09-10.md) and [response-memory](SEARCH_RESPONSE_MEMORY_2026-09-10.md) reports. Deployment does not establish twelve-request all-success capacity or complete the separate cache investigation.

Raw workflow metadata, activation log, post-deployment checks and observer output are retained under ignored `.local/evidence/search-memory-deploy-20260910/`. A documentation-only follow-up records this verification without rebuilding the unchanged application.
