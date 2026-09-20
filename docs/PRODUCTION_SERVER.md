# Production server and branch deployments

Start with [the current deployment and maintenance runbook](DEPLOYMENT_RUNBOOK.md) for release order, environment verification and troubleshooting.

Production target: `160.25.226.236`, `api.shapontravels.com`, Ubuntu 24.04 x86_64.
Development target: the existing `160.25.226.72` server (`sendbox.shapontravels.com`).
The operator requested a new empty database. Do not copy the development database,
supplier credentials, user accounts, or identity activation settings into it.

## Release contents and cleanup

Deploy only the tested `shapontravels-api` binary and `SHA256SUMS` from one successful
GitHub Actions run. Never upload the working tree, `.env`, `.local`, `target`, test
fixtures or diagnostic examples. Runtime assets and SQL migrations are embedded
at build time. Keep all existing migrations and regression fixtures in Git.
The two `legacy_test` sources are referenced under `cfg(test)` and must remain.
Do not delete `.local`: it also contains retained local databases.

## Branch routing

The updated workflow runs checks/builds on `main`, `development`, and `production`.
Only `development` and `production` can deploy. Pull requests cannot deploy.
Deployment enable switches must be **repository variables**, because the job-level
condition is evaluated before environment variables are loaded.

| Branch | GitHub environment | Repository enable variable |
| --- | --- | --- |
| `development` | `development` | `DEVELOPMENT_DEPLOY_ENABLED=true` |
| `production` | `production` | `PRODUCTION_DEPLOY_ENABLED=true` |

Use the following environment variables and secrets. Prefixes are intentional:
missing new settings cannot fall back to old `VPS_*` credentials and send a release
to the other server. GitHub environment names are case-insensitive; an existing
`Production` environment is the same deployment environment as `production`.

| Setting | Production | Development |
| --- | --- | --- |
| Host variable | `PRODUCTION_VPS_HOST=160.25.226.236` | `DEVELOPMENT_VPS_HOST=160.25.226.72` |
| Port variable | `PRODUCTION_VPS_PORT=22` | `DEVELOPMENT_VPS_PORT=22` |
| SSH private key secret | `PRODUCTION_VPS_SSH_PRIVATE_KEY` | `DEVELOPMENT_VPS_SSH_PRIVATE_KEY` |
| Verified host-key secret | `PRODUCTION_VPS_KNOWN_HOSTS` | `DEVELOPMENT_VPS_KNOWN_HOSTS` |

Each environment is restricted to its matching branch. The old
`VPS_DEPLOY_ENABLED` switch is set to `false`, retiring deployment from historical
`main` workflows. Do not turn that switch back on or repoint old generic `VPS_*`
variables/secrets to the new production server. New routing uses only the prefixed
settings above.

As of 2026-09-20 development uses `APP_ENV=uat`, with supplier booking/ticketing
and real notification delivery disabled. Review UAT credentials and execution
controls before enabling supplier mutations. A Git branch alone does not change
the runtime environment (supported values: `local`, `test`, `uat`, `production`).

## Server files

Install the reviewed files as root; do not start the API before a release and
database migrations exist:

- `scripts/deploy/activate.sh` → `/usr/local/sbin/shapontravels-activate`, mode 755.
- `scripts/deploy/backup.sh` → `/usr/local/sbin/shapontravels-backup`, mode 755.
- `scripts/deploy/shapontravels.service` → `/etc/systemd/system/shapontravels.service`.
- `scripts/deploy/shapontravels-backup.service` and `.timer` → `/etc/systemd/system/`.
- `scripts/deploy/nginx-production.conf` → `/etc/nginx/sites-available/shapontravels`.

Use separate `deploy` and `shapontravels` Linux users. The API runs as
`shapontravels`, binds `127.0.0.1:8080`, and reads `/etc/shapontravels/.env`
(root:shapontravels, mode 640). PostgreSQL 18 listens locally on 5432, with separate
`shapon_app` and `shapon_migrator` roles. The migration URL lives in
`/etc/shapontravels/migration-database-url` (root:root, mode 600).
Set `AUTH_TRUSTED_PROXY_IPS=127.0.0.1,::1` for the supplied Nginx config.

Authorize the dedicated Actions public key for the `deploy` user with `restrict`;
grant only `/usr/local/sbin/shapontravels-activate` through passwordless sudo.
The helper checksum must match the release's source. The current helper backs up,
stops the API, migrates, grants runtime privileges, activates, and checks health.
There is restart downtime; it does not automatically roll back database changes.

Allow incoming TCP 22, 80, 443. Keep 5432 and 8080 private. After DNS points to this
host, issue a certificate for `api.shapontravels.com` and redirect HTTP to HTTPS.
Verify certificate renewal and both health endpoints through HTTPS.

Enable `shapontravels-backup.timer` for daily backups around 02:15 UTC. Backups live
in `/var/backups/shapontravels` (postgres:postgres, mode 700), with seven-day local
retention. An independent off-server backup destination and failure notifications
still need configuration; local backups do not protect against losing this VPS.

Migration `0061` qualifies the helper used by the generated import booking
reference column so a dump can restore with PostgreSQL's empty `search_path`.
Use a release containing this migration before relying on fresh backups. Verify
restoration into a disposable database with `pg_restore --exit-on-error
--no-owner --no-privileges`, check the applied migration count, and drop only the
disposable database afterward. Listing a dump's contents alone does not test
whether its schema and data can be restored. Never edit previously applied
migrations; `build.rs` ensures new migration files rebuild the embedded migrator.

## Application activation

As verified on 2026-09-20, production canonical identity is active at revision 5
and its Vercel production frontend is live. The separate `http://localhost:3001`
profile also connects to production. Development canonical identity is active at
revision 3 with its branch-specific Vercel Preview. Both targets run Rust release
`373e5f6` through migration `0062`. Temporary operator capabilities were removed,
maintenance is off and branch auto-deployment is enabled. Production Rust business
notifications are active; development real delivery remains disabled. See
[business notifications](BUSINESS_NOTIFICATIONS.md) for worker ownership and policy.
The generic backend administrator and the portal Super Admin are separate;
canonical mode fences legacy `/admin/login` and other generic admin routes.
See the frontend's [local production connection guide](../../shopontravels/docs/PRODUCTION_API_LOCAL.md).

A healthy empty installation is not a configured travel business. Bootstrap the
real administrator for a new installation, configure supplier accounts/IP allowlists
and pricing, and complete the chosen identity rollout. Keep supplier booking/ticketing
disabled until supplier setup is reviewed. Health checks only
establish process and database/schema readiness.

See [the original server guide](SERVER_SETUP.md) for service administration and
[CI/CD](CI_CD.md) for release-helper behavior. The old server IP and `main` routing
in those historical instructions do not describe this new target.
