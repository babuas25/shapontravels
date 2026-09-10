# Search admission deployment — 2026-09-10

User explicitly authorized commit, push and deployment after reviewing the constrained VPS burst results.

- Application commit: `7c24ab0818f8315cba113e07d75143d2952178de`.
- [GitHub Actions run](https://github.com/babuas25/shapontravels/actions/runs/34474701297): checks, release build and deployment all succeeded.
- CI: formatting and all-target Clippy passed; 43 ordinary tests, the disposable PostgreSQL integration test (46.72 seconds), and five deployment script tests passed.
- Activation confirmed this exact SHA at 12:11:13 UTC (18:11:13 Asia/Dhaka), with live and ready checks passed. Production service is active with new PID 72104.
- Eight independent public HTTPS checks passed: live/ready 200, unauthenticated Search/admin 401, and four OpenAPI compression negotiation cases with identical decoded bytes. The deployed Search OpenAPI includes SEARCH_BUSY in its 503 response description. All checked responses retained no-store and request IDs.
- Read-only observer confirmed the cleanup worker started with 15-minute retention and 30-second interval; eligible cleanup backlog, expired markers, RePrice and booking counts were zero at observation.

The deployed code defaults to four active Search requests, eight waiting slots and a 2000 ms admission timer. No production environment or Nginx configuration was changed in this release. Effective admission startup fields were not independently read: the existing observer exposes cleanup logs only. Exact binary deployment and documented code defaults are verified; do not represent this as an independent environment-override audit.

The earlier constrained VPS twelve-arrival benchmark remains four successes/eight SEARCH_BUSY per wave. Deployment does not establish all-success capacity for twelve simultaneous customers. The two-second timer is admission waiting time, not a full HTTP latency guarantee. No offers were truncated by this change; no supplier Search/Book/Issue calls were made during deployment verification.

Local verification script, HTTPS results and observer output: `.local/evidence/search-admission-deploy-20260910/`. Prior benchmark evidence remains linked in REQUIREMENTS.md. A subsequent documentation-only commit records this evidence without rebuilding the application.
