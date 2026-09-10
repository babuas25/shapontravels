#!/usr/bin/env bash
# Run once from an authenticated VPS root terminal after reviewing observe.sh.
set -euo pipefail
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
[[ $# == 0 ]] || { echo 'No arguments accepted' >&2; exit 2; }
[[ $EUID == 0 ]] || { echo 'Run from the VPS root terminal' >&2; exit 1; }
source_dir=$(cd -- "$(dirname -- "$0")" && pwd)
install -o root -g root -m 0755 "$source_dir/observe.sh" /usr/local/sbin/shapontravels-observe
sudoers_tmp=$(mktemp)
trap 'rm -f -- "$sudoers_tmp"' EXIT
printf '%s\n' 'deploy ALL=(root) NOPASSWD: /usr/local/sbin/shapontravels-observe ""' > "$sudoers_tmp"
visudo -cf "$sudoers_tmp"
install -o root -g root -m 0440 "$sudoers_tmp" /etc/sudoers.d/shapontravels-observe
visudo -cf /etc/sudoers
printf '%s\n' 'Installed read-only cleanup observation helper.'
