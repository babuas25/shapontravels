# Shapon Travels — Ubuntu 24.04 সার্ভার সেটআপ

সার্ভার: `160.25.226.72` · Bengal Cloud BDIX VPS 2 · Ubuntu 24.04 LTS

এই গাইড নতুন VPS-এর জন্য। SSH login ইতিমধ্যে হয়েছে। এখানে দেওয়া সেটআপ এখনও সার্ভারে চালানো বা যাচাই করা হয়নি। প্রতিটি ধাপ সফল হলে পরের ধাপে যাবে; error হলে সেটি ঠিক করে এগোবে।

ব্যবস্থা: Internet → Nginx/HTTPS → Rust/Axum (`127.0.0.1:8080`) → PostgreSQL 18 (`127.0.0.1:5432`)।

`api.example.com` যেখানে আছে নিজের API domain বসাবে। Password বা `.env` কখনো Git-এ বা চ্যাটে দেবে না। নিচের অধিকাংশ কমান্ড **VPS-এর root terminal-এ**; Mac ও অন্য user-এর কমান্ড আলাদাভাবে চিহ্নিত।

## GitHub Actions দিয়ে deployment

স্বয়ংক্রিয় CI/CD ব্যবহার করলে [CI_CD.md](CI_CD.md) অনুসরণ করো। সে ক্ষেত্রে GitHub runner binary build করবে; এই গাইডের manual VPS build/pull ধাপ প্রয়োজন নেই। Database, users, runtime configuration, systemd, Nginx, HTTPS ও backup সেটআপ এখানকার নির্দেশনা অনুযায়ী হবে।

## 1. Ubuntu ও firewall

VPS-এ:

```bash
apt update
apt upgrade -y
apt install -y nginx ufw curl ca-certificates build-essential pkg-config libssl-dev unzip rsync git cron
systemctl enable --now cron
```

### Package configuration স্ক্রিন এলে

`apt upgrade` বা package installation চলার সময় রঙিন **Package configuration** স্ক্রিন আসতে পারে। এটি error নয়; keyboard দিয়ে option বাছাই করতে হয়।

**Configuring console-setup → Character set to support** স্ক্রিনে:

1. **Guess optimal character set** নির্বাচিত রাখো। অন্য option নির্বাচিত থাকলে ↑/↓ দিয়ে এটি বেছে নাও।
2. **Tab** চেপে নিচের **<Ok>** button-এ যাও।
3. **Enter** চাপো। Installation আবার চলবে অথবা পরের configuration প্রশ্ন আসবে।

এই setting সার্ভারের console font-এর জন্য; ওয়েবসাইট/API-এর বাংলা text বা database encoding সেট করে না।

এ ধরনের dialog-এ সাধারণত ↑/↓ দিয়ে option বাছাই, Tab দিয়ে button বদলানো এবং Enter দিয়ে নিশ্চিত করা যায়। Checkbox-এর তালিকা হলে Space দিয়ে tick বদলানো যায়। অন্য কোনো configuration প্রশ্ন এলে তার বিষয় দেখে সিদ্ধান্ত নেবে—বিশেষ করে SSH configuration replace করার প্রশ্নে না পড়ে Enter দেবে না। Installation শেষ হয়ে shell prompt ফিরে এলে পরের কমান্ড চালাবে।

বর্তমান SSH port 22। অন্য port ব্যবহার করলে সেটি allow করার পরে firewall চালাবে।

```bash
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable
ufw status
systemctl enable --now nginx
```

বর্তমান SSH session খোলা রেখে Mac-এর আরেক Terminal থেকে `ssh root@160.25.226.72` চালিয়ে login যাচাই করো। Bengal Cloud panel firewall থাকলে সেখানেও এই তিনটি port allow রাখো। 5432 ও 8080 public করো না।

Kernel update-এর পরে `/var/run/reboot-required` থাকলে সুবিধামতো `reboot` করে আবার SSH login করো।

## 2. আলাদা Linux users ও directories

`deploy` দিয়ে build হবে; `shapontravels` দিয়ে শুধু API চলবে।

```bash
adduser deploy
adduser --system --group --home /opt/shapontravels --no-create-home shapontravels
install -d -o deploy -g deploy -m 750 /opt/shapontravels-src
install -d -o root -g shapontravels -m 750 /opt/shapontravels
install -d -o root -g shapontravels -m 750 /etc/shapontravels
```

এগুলো নতুন installation-এর কমান্ড; user আগে থাকলে পুনরায় তৈরি করবে না।

## 3. PostgreSQL 18

Ubuntu-এর default package-এর পরিবর্তে PostgreSQL-এর official repository থেকে 18 ইনস্টল:

```bash
apt install -y postgresql-common
/usr/share/postgresql-common/pgdg/apt.postgresql.org.sh
apt update
apt install -y postgresql-18 postgresql-client-18
pg_lsclusters
```

`18 main` cluster `online` এবং port `5432` আছে নিশ্চিত করো। অন্য cluster আগে থাকলে সেটি মুছবে না; port/configuration আগে পর্যালোচনা করো।

```bash
sudo -u postgres psql -p 5432 -c 'SHOW listen_addresses;'
```

নতুন installation-এ `localhost` থাকা উচিত। Database internet-এ expose করবে না।

দুটি database role তৈরি করো। নিচের interactive `psql` session-এ `\password` নিজে password চাইবে; shell history-তে password লিখতে হবে না।

```bash
sudo -u postgres psql -p 5432
```

```sql
CREATE ROLE shapon_migrator LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
\password shapon_migrator
CREATE ROLE shapon_app LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
\password shapon_app
CREATE DATABASE shapontravels OWNER shapon_migrator;
REVOKE ALL ON DATABASE shapontravels FROM PUBLIC;
GRANT CONNECT ON DATABASE shapontravels TO shapon_app;
\connect shapontravels
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO shapon_app;
\q
```

দুটি আলাদা শক্তিশালী password password manager-এ রাখো। Database URL-এ ব্যবহারের সুবিধায় দীর্ঘ random alphanumeric password নেওয়া যায়। অন্য special character থাকলে URL-এ password অংশ percent-encode করতে হবে।

## 4. GitHub repository থেকে build

Repository: `https://github.com/babuas25/shapontravels`। প্রথম commit/push Mac থেকে করা হবে; VPS শুধু repository পড়বে। `.env`, `.local/` ও `target/` Git-এ যাবে না।

প্রথম push করতে হলে **Mac Terminal-এ** (আগে push হয়ে থাকলে এই block এড়িয়ে যাও):

```bash
cd /Users/ashifbabu/Projects/shapontravels
git status
git push -u origin main
```

**VPS root terminal-এ**, section 2-এর users তৈরি হওয়ার পরে:

```bash
su - deploy
ssh-keygen -t ed25519 -C "shapontravels-vps-readonly" -f ~/.ssh/shapontravels_github
```

Key আগে থাকলে overwrite করবে না। Automated read-only fetch-এর জন্য passphrase ফাঁকা রাখতে পারো। Public key দেখো:

```bash
cat ~/.ssh/shapontravels_github.pub
```

GitHub repository → **Settings → Deploy keys → Add deploy key**। Title `Shapon Travels VPS` এবং উপরের public key দাও। **Allow write access tick করবে না**। Private key (যে file-এর শেষে `.pub` নেই) কখনো শেয়ার করবে না।

নতুন deploy user-এর SSH config তৈরি করো; আগে config থাকলে নিচের host block সেটিতে যোগ করবে:

```bash
install -d -m 700 ~/.ssh
cat >> ~/.ssh/config <<'EOF'
Host github-shapontravels
    HostName github.com
    User git
    IdentityFile ~/.ssh/shapontravels_github
    IdentitiesOnly yes
EOF
chmod 600 ~/.ssh/config
ssh -T github-shapontravels
```

প্রথম সংযোগে fingerprint দেখালে [GitHub-এর official fingerprints](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints)-এর সঙ্গে মিলিয়ে `yes` দাও। Successful authentication-এর পরে GitHub shell access দেয় না—এই বার্তা স্বাভাবিক; command exit code 1-ও হতে পারে।

Section 2-তে তৈরি source directory খালি থাকা অবস্থায়:

```bash
git clone git@github-shapontravels:babuas25/shapontravels.git /opt/shapontravels-src
cd /opt/shapontravels-src
git log -1 --oneline
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/shapon-rustup.sh
sh /tmp/shapon-rustup.sh -y --profile minimal --default-toolchain none
. "$HOME/.cargo/env"
rustup show
cargo build --release --locked -j 1
exit
```

Source directory খালি না থাকলে clone করার আগে পরীক্ষা করো; ফাইল মুছে ফেলবে না। Repository-র `rust-toolchain.toml` অনুযায়ী toolchain install হবে। Version unavailable হলে error সমাধান করো; ইচ্ছামতো version বদলাবে না। 4 GB RAM-এ build parallelism কমাতে `-j 1` দেওয়া হয়েছে। Mac-এর binary Ubuntu-তে চলে না; Linux build প্রয়োজন।

আবার **root shell-এ**:

```bash
install -o root -g shapontravels -m 750 /opt/shapontravels-src/target/release/shapontravels-api /opt/shapontravels/shapontravels-api
```

## 5. Runtime configuration

শুধু প্রথমবার:

```bash
install -o root -g shapontravels -m 640 /opt/shapontravels-src/.env.example /etc/shapontravels/.env
nano /etc/shapontravels/.env
```

এই মানগুলো ঠিক করো; `YOUR_APP_DB_PASSWORD`-এর জায়গায় `shapon_app`-এর password:

```dotenv
DATABASE_URL=postgres://shapon_app:YOUR_APP_DB_PASSWORD@127.0.0.1:5432/shapontravels
APP_ENV=production
APP_BIND=127.0.0.1:8080
DB_MAX_CONNECTIONS=10
DB_TIMEOUT_SECONDS=5
```

বাকি supplier settings `.env.example` অনুযায়ী রাখবে। আসল supplier URL/email/password শুধু এই private file-এ যোগ করো। Placeholder URLs দিয়ে health check চললেও বাস্তব flight search চলবে না। Password-এ dotenv বিশেষ character থাকলে যথাযথ quoting ব্যবহার করো।

অ্যাপ working directory থেকে dotenv নিজে পড়বে; এই file-কে shell script হিসেবে `source` করবে না।

## 6. Migrations ও runtime permissions

Migration credentials API service-কে দেওয়া হবে না। Root terminal-এ temporary environment variable দিয়ে migration চালাও:

```bash
cd /etc/shapontravels
read -rsp 'Migration DATABASE_URL (shapon_migrator role): ' DATABASE_URL
echo
export DATABASE_URL
/opt/shapontravels/shapontravels-api migrate
unset DATABASE_URL
```

Prompt-এ দেবে `postgres://shapon_migrator:YOUR_MIGRATOR_PASSWORD@127.0.0.1:5432/shapontravels`। এটি terminal-এ দেখা যাবে না। Migration success message না এলে পরের ধাপে যেও না।

অ্যাপকে প্রয়োজনীয় data permissions দাও। **প্রতিবার নতুন migrations চালানোর পর এই block চালাতে হবে**, যাতে নতুন tables-এর permissions যোগ হয়:

```bash
sudo -u postgres psql -p 5432 -d shapontravels -v ON_ERROR_STOP=1 <<'SQL'
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO shapon_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO shapon_app;
REVOKE INSERT, UPDATE, DELETE ON TABLE _sqlx_migrations FROM shapon_app;
REVOKE UPDATE, DELETE ON TABLE audit_events, markup_rule_versions FROM shapon_app;
SQL
```

`shapon_app` database owner নয় এবং DDL/TRUNCATE permissions দেওয়া হচ্ছে না। Audit tables-এর immutability triggers-ও থাকবে। ভবিষ্যৎ schema-তে নতুন immutable table এলে grants review করতে হবে।

প্রথম Super Admin তৈরি:

```bash
cd /etc/shapontravels
sudo -u shapontravels /opt/shapontravels/shapontravels-api bootstrap-admin
```

নিজের username ও 12–256 bytes password দাও। Bootstrap একবারই হয়। Test account ব্যবহার করো না।

## 7. systemd service

Root terminal-এ:

```bash
cat > /etc/systemd/system/shapontravels.service <<'EOF'
[Unit]
Description=Shapon Travels API
Wants=network-online.target
After=network-online.target postgresql.service

[Service]
Type=simple
User=shapontravels
Group=shapontravels
WorkingDirectory=/etc/shapontravels
ExecStart=/opt/shapontravels/shapontravels-api serve
Restart=on-failure
RestartSec=5
TimeoutStopSec=150
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
UMask=0077

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now shapontravels
systemctl status shapontravels --no-pager
curl --fail http://127.0.0.1:8080/health/live
curl --fail http://127.0.0.1:8080/health/ready
```

দুই health check-এ HTTP 200 দরকার। `/health/ready` database/migrations যাচাই করে; supplier credentials বা booking readiness যাচাই করে না।

## 8. Domain ও Nginx

Domain DNS-এ নিজের API subdomain-এর **A record → `160.25.226.72`** করো। IPv6 কনফিগার না করলে ওই hostname-এ AAAA record দিও না। প্রথমবার HTTPS সেটআপের সময় সরাসরি DNS ব্যবহার করা সহজ।

নিচের config-এ `api.example.com` বদলে নিজের domain লিখবে:

```bash
nano /etc/nginx/sites-available/shapontravels
```

```nginx
server {
    listen 80;
    server_name api.example.com;

    # Query strings বা passenger data access log-এ সংরক্ষণ না করা।
    access_log off;
    client_max_body_size 2m;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_connect_timeout 5s;
        proxy_read_timeout 150s;
    }
}
```

```bash
ln -s /etc/nginx/sites-available/shapontravels /etc/nginx/sites-enabled/shapontravels
nginx -t
systemctl reload nginx
```

`nginx -t` fail হলে reload করবে না। Symlink আগে থাকলে আবার তৈরি করবে না।

## 9. HTTPS certificate

নিজের domain-এর DNS এই VPS-এ পৌঁছানোর পরে, domain বদলে চালাও:

```bash
apt install -y certbot python3-certbot-nginx
certbot --nginx -d api.example.com --redirect
certbot renew --dry-run
systemctl list-timers --all | grep certbot
```

নিজের email দাও এবং certificate terms পড়ো। যাচাই:

```bash
curl --fail https://api.example.com/health/live
curl --fail https://api.example.com/health/ready
```

Swagger: `https://api.example.com/docs/`। HTTPS চালু হওয়ার আগে internet দিয়ে admin password বা machine token পাঠাবে না।

## 10. দৈনিক database backup

প্রথমে VPS-এ daily backup; পরে অবশ্যই VPS-এর বাইরেও encrypted copy রাখবে। একই VPS-এর backup VPS হারালে কাজে আসবে না।

```bash
install -d -o postgres -g postgres -m 700 /var/backups/shapontravels
cat > /usr/local/sbin/shapontravels-backup <<'EOF'
#!/bin/bash
set -euo pipefail
umask 077
backup_path="/var/backups/shapontravels/db-$(date -u +%Y%m%dT%H%M%SZ).dump"
trap 'rm -f "${backup_path}.tmp"' EXIT
/usr/lib/postgresql/18/bin/pg_dump -p 5432 -Fc shapontravels > "${backup_path}.tmp"
mv "${backup_path}.tmp" "$backup_path"
find /var/backups/shapontravels -name 'db-*.dump' -type f -mtime +6 -delete
EOF
chmod 755 /usr/local/sbin/shapontravels-backup
cat > /etc/cron.d/shapontravels-backup <<'EOF'
15 2 * * * postgres /usr/local/sbin/shapontravels-backup
EOF
chmod 644 /etc/cron.d/shapontravels-backup
sudo -u postgres /usr/local/sbin/shapontravels-backup
ls -lh /var/backups/shapontravels
timedatectl
```

এটি server timezone অনুযায়ী 02:15-এ চলে। 25 GB disk-এ সাত দিনের backup-ও বেশি হতে পারে—database বড় হলে retention কমিয়ে off-server storage ব্যবহার করো। Backup failure monitoring ও off-server destination production চালুর আগে ঠিক করতে হবে।

নিজের Mac-এ একটি copy নিতে **Mac Terminal-এ**:

```bash
mkdir -p ~/Backups/shapontravels
chmod 700 ~/Backups/shapontravels
scp 'root@160.25.226.72:/var/backups/shapontravels/db-*.dump' ~/Backups/shapontravels/
chmod 600 ~/Backups/shapontravels/*.dump
```

এটি manual copy; স্বয়ংক্রিয় off-server backup নয়। Database backup ছাড়াও `.env` ও database role credentials password manager/encrypted storage-এ রাখো। একটি আলাদা disposable PostgreSQL database/server-এ dump restore করে যাচাই করো; production database-এ restore test চালাবে না।

## 11. নতুন version deploy

1. বর্তমান database backup নাও এবং off-server copy রাখো।
2. Mac থেকে reviewed changes commit/push করো। VPS-এ `su - deploy`, তারপর `cd /opt/shapontravels-src`, `git status --short` চালাও। Local changes থাকলে আগে সমাধান করো; clean হলে `git pull --ff-only origin main` এবং `cargo build --release --locked -j 1` চালাও। `exit` দিয়ে root shell-এ ফিরো। Pull conflict হলে force/reset করবে না।
3. Build সফল হলে maintenance window-তে API stop করো: `systemctl stop shapontravels`।
4. পুরোনো binary সংরক্ষণ: `cp -p /opt/shapontravels/shapontravels-api /opt/shapontravels/shapontravels-api.previous`।
5. নতুন binary `install` করো। Section 6 অনুযায়ী নতুন binary দিয়ে migrations এবং grants চালাও।
6. `systemctl start shapontravels` করে local ও HTTPS health checks চালাও।

Migration fail হলে service বন্ধ রেখে error তদন্ত করো। Schema বদলানোর পরে শুধু পুরোনো binary ফেরানো সবসময় নিরাপদ নয়; rollback-এর আগে schema compatibility যাচাই করতে হবে। পুরোনো migration SQL edit করবে না।

## 12. সমস্যা হলে

```bash
systemctl status shapontravels --no-pager
journalctl -u shapontravels -n 100 --no-pager
pg_lsclusters
nginx -t
df -h
free -h
ss -ltnp
```

| লক্ষণ | কী দেখবে |
| --- | --- |
| SSH permission denied | VPS-এর username/password; Mac বা panel account password নয় |
| 502 Bad Gateway | API service চলছে কি না; local health check |
| Database connection failed | `.env` URL, DB password, cluster port ও status |
| Schema is not current | নতুন binary দিয়ে explicit `migrate`, তারপর grants |
| Search কাজ করছে না | supplier credentials, API entitlement/IP allowlist, supplier controls ও active markup |
| Certificate issue | domain A/AAAA record, public port 80/443, DNS propagation |
| Build killed / disk full | RAM/disk usage; `-j 1`; প্রয়োজনে অন্য Linux builder |

Nginx error logs-এ request metadata থাকতে পারে; log শেয়ার করার আগে sensitive data সরাও। `.env` বা password দেখানোর কমান্ড চালিয়ে output শেয়ার করবে না।

## 13. Go-live যাচাই

- [ ] HTTPS এবং certificate renewal dry run সফল।
- [ ] Reboot-এর পরে service ফিরে আসে ও উভয় health endpoint 200 দেয়।
- [ ] PostgreSQL/API public interface-এ শুনছে না।
- [ ] নিজের Super Admin এবং প্রয়োজনীয় API client তৈরি হয়েছে।
- [ ] Supplier settings, IP allowlist ও markup rules যাচাই হয়েছে।
- [ ] Search/Reprice/Booking-এর বর্তমান contract ও limitations পড়ে প্রয়োজনীয় flow পরীক্ষা হয়েছে; deployment সফল হওয়া মানেই ticketing প্রস্তুত নয়।
- [ ] দৈনিক backup, off-server encrypted copy ও isolated restore test প্রস্তুত।
- [ ] Disk, service health, backup failures পর্যবেক্ষণ এবং log retention নির্ধারিত।
- [ ] SSH key login দ্বিতীয় session-এ পরীক্ষা করা হয়েছে; তারপর প্রয়োজনমতো password/root login policy শক্ত করা হয়েছে।

## Reference

- [PostgreSQL Ubuntu repository](https://www.postgresql.org/download/linux/ubuntu/)
- [Ubuntu firewall](https://documentation.ubuntu.com/server/how-to/security/firewalls/index.html)
- [Rust installation](https://www.rust-lang.org/tools/install)
- [Certbot instructions](https://certbot.eff.org/instructions?ws=nginx&os=pip)
- [Project README](../README.md)
- [Authentication](AUTHENTICATION.md)
- [Search API](SEARCH_API.md)
- [Reprice API](REPRICE_API.md)
- [Booking API](BOOKING_API.md)
