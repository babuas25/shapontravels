#!/usr/bin/env bash
set -euo pipefail
: "${VPS_HOST:?Set VPS_HOST}"
: "${VPS_SSH_PRIVATE_KEY:?Set VPS_SSH_PRIVATE_KEY}"
: "${VPS_KNOWN_HOSTS:?Set VPS_KNOWN_HOSTS}"
: "${DEPLOY_SHA:?Set DEPLOY_SHA}"
VPS_PORT=${VPS_PORT:-22}
[[ $DEPLOY_SHA =~ ^[0-9a-f]{40}$ ]] || { echo 'Invalid commit SHA' >&2; exit 1; }
[[ $VPS_HOST =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*$ ]] || { echo 'Use an IPv4 address or hostname' >&2; exit 1; }
[[ $VPS_PORT =~ ^[0-9]+$ ]] || exit 1
helper_sha=$(sha256sum "$(dirname "$0")/activate.sh")
helper_sha=${helper_sha%% *}
cd dist
sha256sum --check SHA256SUMS
ssh_dir=$(mktemp -d)
trap 'rm -rf "$ssh_dir"' EXIT
chmod 700 "$ssh_dir"
printf '%s\n' "$VPS_SSH_PRIVATE_KEY" > "$ssh_dir/key"
printf '%s\n' "$VPS_KNOWN_HOSTS" > "$ssh_dir/known_hosts"
chmod 600 "$ssh_dir/key" "$ssh_dir/known_hosts"
unset VPS_SSH_PRIVATE_KEY VPS_KNOWN_HOSTS
ssh_options=(-i "$ssh_dir/key" -o BatchMode=yes -o IdentitiesOnly=yes
  -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$ssh_dir/known_hosts"
  -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=4)
# The helper is installed separately by a server administrator. A stale helper
# can omit required database grants even when the application migration passes.
# Check before upload/activation so a mismatch leaves the running API untouched.
installed_helper=$(ssh "${ssh_options[@]}" -p "$VPS_PORT" "deploy@$VPS_HOST" \
  'sha256sum /usr/local/sbin/shapontravels-activate')
[[ ${installed_helper%% *} == "$helper_sha" ]] || {
  echo 'Deployment helper differs from this release. Install scripts/deploy/activate.sh as root before retrying.' >&2
  exit 1
}
ssh "${ssh_options[@]}" -p "$VPS_PORT" "deploy@$VPS_HOST" \
  "mkdir -p /home/deploy/incoming/$DEPLOY_SHA"
scp "${ssh_options[@]}" -P "$VPS_PORT" shapontravels-api SHA256SUMS \
  "deploy@$VPS_HOST:/home/deploy/incoming/$DEPLOY_SHA/"
ssh "${ssh_options[@]}" -p "$VPS_PORT" "deploy@$VPS_HOST" \
  "cd /home/deploy/incoming/$DEPLOY_SHA && sha256sum --check SHA256SUMS && sudo -n /usr/local/sbin/shapontravels-activate $DEPLOY_SHA"
