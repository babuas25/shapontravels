#!/bin/sh
set -eu
PROJECT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$PROJECT_DIR"
export PATH="$HOME/.cargo/bin:$PROJECT_DIR/.local/pgsql/bin:$PATH"
if [ ! -x "$PROJECT_DIR/.local/pgsql/bin/pg_ctl" ]; then
    echo 'Local PostgreSQL build missing; use your PostgreSQL installation and set DATABASE_URL.' >&2
    exit 1
fi
mkdir -p "$PROJECT_DIR/.local/pgsocket"
chmod 700 "$PROJECT_DIR/.local/pgsocket"
if ! pg_ctl -D "$PROJECT_DIR/.local/pgdata" status >/dev/null 2>&1; then
    pg_ctl -D "$PROJECT_DIR/.local/pgdata" -l "$PROJECT_DIR/.local/postgres.log" -o "-h '' -k $PROJECT_DIR/.local/pgsocket -p 55432" start
fi
export DATABASE_URL="postgres://$(id -un)@localhost:55432/shapontravels?host=$PROJECT_DIR/.local/pgsocket"
exec cargo run --locked -- "${1:-serve}"
