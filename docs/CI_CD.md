# GitHub Actions CI/CD — Bengal Cloud VPS

Repository: [babuas25/shapontravels](https://github.com/babuas25/shapontravels)

## কী হবে

- Pull request: formatting, Clippy, unit/fixture tests, disposable PostgreSQL 18 integration tests, deployment failure tests, তারপর Ubuntu 24.04 x86_64 release build।
- `main` push: একই checks/build; deployment enabled থাকলে সেই run-এর binary VPS-এ যায়।
- Actions → **CI and VPS deployment → Run workflow → main** দিয়েও পুরো pipeline চালানো যায়।
- Database/supplier passwords VPS-এই থাকে। CI-তে real supplier tests বা booking/ticketing calls চলে না।
- GitHub-এ build হয় বলে VPS-এ Rust compiler বা source checkout প্রয়োজন নেই। Source GitHub-এ version-controlled; deploy হয় সেই commit থেকে তৈরি binary।

Workflow: [ci.yml](../.github/workflows/ci.yml)। এখন deployment বন্ধ থাকবে যতক্ষণ repository variable `VPS_DEPLOY_ENABLED=true` না করছ। Server setup অসম্পূর্ণ থাকলে এটি চালু করবে না।

## 1. VPS একবার প্রস্তুত করো

[SERVER_SETUP.md](SERVER_SETUP.md)-এর sections 1–3 অনুযায়ী packages, Linux users, PostgreSQL 18 ও database roles তৈরি করো। `git`, `python3`, `sudo`, `curl`, `util-linux` থাকতে হবে:

```bash
apt install -y git python3 sudo curl util-linux
```

Section 4-এর VPS build/deploy-key ধাপ CI/CD ব্যবহারে প্রয়োজন নেই। Runtime-এর জন্য section 5-এর `.env`, section 7-এর systemd unit এবং section 10-এর backup helper তৈরি করতে হবে। Binary/database schema আসার আগে service start ও bootstrap-admin চালাবে না।

Repository থেকে reviewed configuration/scripts নিতে Mac-এর clone ব্যবহার করো। **Mac Terminal-এ**:

```bash
cd /Users/ashifbabu/Projects/shapontravels
scp scripts/deploy/activate.sh .env.example root@160.25.226.72:/tmp/
```

**VPS root shell-এ**, শুধু প্রথমবার `.env` তৈরি:

```bash
install -o root -g shapontravels -m 640 /tmp/.env.example /etc/shapontravels/.env
nano /etc/shapontravels/.env
```

Runtime DATABASE_URL-এ `shapon_app` credentials, `APP_ENV=production`, `APP_BIND=127.0.0.1:8080` এবং supplier settings বসাও। Existing `.env` overwrite করবে না।

Migration URL-এর আলাদা root-only file:

```bash
install -o root -g root -m 600 /dev/null /etc/shapontravels/migration-database-url
nano /etc/shapontravels/migration-database-url
```

এতে এক লাইন থাকবে—`DATABASE_URL=` prefix বা quote নয়:

```text
postgres://shapon_migrator:YOUR_MIGRATOR_PASSWORD@127.0.0.1:5432/shapontravels
```

নিজের password বসাবে; special characters URL-encode করবে। এই file GitHub-এ যাবে না।

Root-owned deployment helper ও সীমিত sudo entry install:

```bash
install -o root -g root -m 755 /tmp/activate.sh /usr/local/sbin/shapontravels-activate
install -d -o deploy -g deploy -m 700 /home/deploy/incoming
printf '%s\n' 'deploy ALL=(root) NOPASSWD: /usr/local/sbin/shapontravels-activate' > /etc/sudoers.d/shapontravels-deploy
chmod 440 /etc/sudoers.d/shapontravels-deploy
visudo -cf /etc/sudoers.d/shapontravels-deploy
systemctl daemon-reload
systemctl enable shapontravels
```

Helper নিজের argument হিসেবে শুধু পূর্ণ 40-character commit SHA গ্রহণ করে। এটি binary-কে root হিসেবে চালায় না; migration/API চলে `shapontravels` user হিসেবে। তবে deployment access মানে trusted application code ও schema migrations চালানোর ক্ষমতা—repository write access ও SSH key শুধু trusted maintainers রাখবে।

Section 10 অনুযায়ী backup helper চালিয়ে যাচাই করো। প্রথম deployment-এর আগে empty database-এর backup নেওয়াও স্বাভাবিক। Sudo validation, config, backup বা service unit-এ error থাকলে আগে ঠিক করো।

## 2. GitHub Actions → VPS SSH key

এটি VPS থেকে GitHub clone করার deploy key-এর বিপরীত দিকের connection। **এই key repository-এর Deploy keys-এ যোগ করবে না।**

নিজের **Mac Terminal-এ** নতুন dedicated key তৈরি:

```bash
mkdir -p ~/.ssh
chmod 700 ~/.ssh
ssh-keygen -t ed25519 -C "github-actions-shapontravels" -f ~/.ssh/shapontravels_actions
```

Automation-এর জন্য passphrase prompt-এ Enter দিয়ে ফাঁকা রাখো। একই নামে key আগে থাকলে overwrite করবে না।

Public key VPS-এ পাঠাও:

```bash
scp ~/.ssh/shapontravels_actions.pub root@160.25.226.72:/tmp/shapontravels_actions.pub
```

**VPS root terminal-এ**:

```bash
install -d -o deploy -g deploy -m 700 /home/deploy/.ssh
{ printf 'restrict '; cat /tmp/shapontravels_actions.pub; } >> /home/deploy/.ssh/authorized_keys
chown deploy:deploy /home/deploy/.ssh/authorized_keys
chmod 600 /home/deploy/.ssh/authorized_keys
rm /tmp/shapontravels_actions.pub
```

`restrict` forwarding ও PTY বন্ধ রাখে; deploy-এর SSH/scp commands চলতে পারে।

### Server host key যাচাই

VPS-এর authenticated root session-এ:

```bash
ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub
printf '160.25.226.72 '
cat /etc/ssh/ssh_host_ed25519_key.pub
```

শেষ দুই command-এর output মিলে একটি line হবে: `160.25.226.72 ssh-ed25519 AAAA...`। সেটি `VPS_KNOWN_HOSTS` secret-এ যাবে। এটি public host key; কোনো private host key পড়বে না। Custom SSH port হলে hostname অংশ `[160.25.226.72]:PORT` হবে। Workflow host verification বন্ধ করে না এবং unverified `ssh-keyscan` ব্যবহার করে না।

## 3. GitHub variables ও secrets

Repository → **Settings → Secrets and variables → Actions**।

**Variables** tab-এ repository variables:

| Name | Value |
| --- | --- |
| `VPS_HOST` | `160.25.226.72` |
| `VPS_PORT` | `22` |
| `VPS_DEPLOY_ENABLED` | প্রথমে `false`; server ready হলে `true` |

**Secrets** tab-এ:

| Name | Value |
| --- | --- |
| `VPS_SSH_PRIVATE_KEY` | Mac-এর `~/.ssh/shapontravels_actions`-এর সম্পূর্ণ private key |
| `VPS_KNOWN_HOSTS` | আগের ধাপে VPS থেকে পাওয়া verified host-key line |

Mac-এ private key terminal output-এ না দেখিয়ে clipboard-এ নিতে:

```bash
pbcopy < ~/.ssh/shapontravels_actions
```

GitHub-এর secret field-এ paste করো। চ্যাটে বা repository file-এ paste করবে না। কাজ শেষে clipboard পরিষ্কার করতে `printf '' | pbcopy` চালাতে পারো।

**Settings → Environments → production** তৈরি করে deployment branch `main`-এ সীমিত রাখো যেখানে তোমার GitHub plan এটি সমর্থন করে। Workflow-ও main ছাড়া deployment চালাবে না। Environment secrets সমর্থিত হলে secrets সেখানে রাখা যায়; enable variable repository-level-এ রাখতে হবে, কারণ job condition environment load হওয়ার আগে evaluate হয়।

যারা main-এ push করতে পারে তারা deployment চালাতে পারে। Repository branch protection-এ PR ও checks বাধ্যতামূলক করা ভালো। Automatic deployment চাইলে আলাদা required-reviewer gate প্রয়োজন নেই।

## 4. প্রথম deployment

1. Server prerequisites, SSH key ও GitHub settings শেষ করো।
2. `VPS_DEPLOY_ENABLED` repository variable `true` করো।
3. Actions → **CI and VPS deployment → Run workflow**, branch **main**।
4. Checks → build → deploy সবুজ হওয়া পর্যন্ত দেখো।
5. VPS-এ যাচাই:

```bash
cat /opt/shapontravels/deployed-sha
systemctl status shapontravels --no-pager
curl --fail http://127.0.0.1:8080/health/ready
```

প্রথম সফল deployment migrations চালিয়ে schema তৈরি করবে। এরপর **একবার** নিজের Super Admin তৈরি করো:

```bash
cd /etc/shapontravels
sudo -u shapontravels /opt/shapontravels/shapontravels-api bootstrap-admin
```

তারপর SERVER_SETUP.md-এর Nginx/domain/HTTPS steps সম্পন্ন করো। Health checks supplier readiness বা বাইরের HTTPS route যাচাই করে না; নিজের HTTPS domain থেকেও ready endpoint পরীক্ষা করো।

## 5. প্রতিবার deployment-এ কী হয়

1. Tests পাস হওয়ার পরে GitHub runner-এ release build ও SHA-256 checksum তৈরি।
2. একই run-এর artifact verified SSH connection দিয়ে upload।
3. Server deployment lock নেয়; binary root-owned release directory-তে কপি হয়।
4. PostgreSQL backup সফল হওয়ার পরে API stop হয়।
5. আলাদা migration credentials দিয়ে migrations, তারপর runtime database grants।
6. Binary link atomically বদলে systemd service start।
7. `/health/live` ও `/health/ready` সফল হলে deployed commit লিখে upload directory পরিষ্কার।

এটি single-server deployment; migrations ও restart চলাকালে কিছু downtime হবে। GitHub pipeline একই branch-এর চলমান run বাতিল করে না; server lock-ও overlapping deployment আটকায়। Superseded pending runs GitHub বাদ দিতে পারে।

একই SHA পুনরায় deploy করা যায়। Old release directories আপাতত রাখা হয়; নিয়মিত disk usage দেখে বর্তমান release ও প্রয়োজনীয় rollback candidate ছাড়া পুরোনোগুলো পর্যালোচনা করে সরাও। 25 GB disk-এ binary/backup retention গুরুত্বপূর্ণ।

## 6. Failure ও recovery

- CI/build fail → deployment হয় না।
- Missing credentials/SSH fail → server activation হয় না।
- Backup fail → চলমান API stop হয় না।
- Migration/grants fail → service বন্ধ থাকে; log ও schema পরীক্ষা করে fix-forward করো।
- Health fail → নতুন service বন্ধ করে workflow failure দেয়। Database পরিবর্তন হয়ে থাকতে পারে বলে automatic binary rollback করা হয় না।

```bash
journalctl -u shapontravels -n 100 --no-pager
pg_lsclusters
df -h
```

প্রয়োজনে `VPS_DEPLOY_ENABLED=false` করে পরবর্তী auto deploy বন্ধ করো। Migration চলার সময় GitHub run manually cancel করবে না। SSH interruption বা run timeout হলে আগে server-এর service, migration state ও deployment lock দেখবে। Confirmed compatible schema ছাড়া পুরোনো binary চালাবে না এবং production database-এ blind restore করবে না।

Deployment helper বদলালে reviewed নতুন `activate.sh` আবার root হিসেবে install করতে হবে; pipeline নিজে privileged helper/sudo policy আপডেট করে না।

## References

- [GitHub deployments and concurrency](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/control-deployments)
- [GitHub workflow artifacts](https://docs.github.com/en/enterprise-cloud%40latest/actions/tutorials/store-and-share-data)
