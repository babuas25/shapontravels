# Repository workflow

- Before deployment, Vercel configuration or identity rollout work, read [the deployment and maintenance runbook](docs/DEPLOYMENT_RUNBOOK.md). Follow its environment selection, backend readiness, coordinated rollout and post-deployment verification steps. Update the release record after changes.

User preference recorded on 2026-09-20:

- This repository (`shapontravels`) is the Rust backend.
- The related frontend is `/Users/ashifbabu/Projects/shopontravels`.
- New backend and frontend work starts on `development`. Backend `development` deploys to `160.25.226.72`; frontend `development` uses its branch-specific Vercel Preview and that development API.
- Production branches remain backend `production` and frontend `main`. Merge tested changes there only for a user-authorized production release.
- When asked to commit and push project changes, push only the backend.
- On 2026-09-20 the user explicitly authorized frontend pushes to the new private repository `https://github.com/babuas25/shapontravels-frontend` when requested.
- Do not push frontend branches, tags, or commits to the old `babuas25/shopontravels` repository or other remotes without explicit authorization. A general backend push request does not include the frontend.
- Do not infer permission to deploy services or apply production database migrations from a Git push request.
