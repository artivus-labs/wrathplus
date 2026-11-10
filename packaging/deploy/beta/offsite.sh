#!/bin/sh
# Encrypt the newest local backup and upload it to an off-box S3-compatible bucket (Cloudflare R2
# on the beta); keep 14 days there. Runs after each wrathplus-backup.service (systemd drop-in).
# Configured only when /etc/wrathplus-beta/offsite.env exists, otherwise a no-op:
#   offsite.env (0600): R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_ENDPOINT, R2_BUCKET
#   offsite.agepub:     the age recipient; its private key is NOT on the box
# Needs age and rclone (apt-get install age rclone).
set -eu
conf=/etc/wrathplus-beta
[ -f "$conf/offsite.env" ] || { echo "offsite: $conf/offsite.env missing; off-box copy skipped"; exit 0; }
. "$conf/offsite.env"
recipient=$(cat "$conf/offsite.agepub")
export RCLONE_CONFIG_R2_TYPE=s3 RCLONE_CONFIG_R2_PROVIDER=Cloudflare RCLONE_CONFIG_R2_REGION=auto
export RCLONE_CONFIG_R2_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" RCLONE_CONFIG_R2_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export RCLONE_CONFIG_R2_UPLOAD_CUTOFF=4G
export RCLONE_CONFIG_R2_ENDPOINT="$R2_ENDPOINT" RCLONE_CONFIG_R2_NO_CHECK_BUCKET=true
dir=${BACKUPS:-/var/backups/wrathplus}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# The newest SQL dump and the newest copy of the account service files (keys, SQLite).
sql=$(cd "$dir" && ls -t wrathplus-beta-*.sql.gz 2>/dev/null | head -1)
acct=$(cd "$dir" && ls -t account-service-*.tar.gz 2>/dev/null | head -1)
[ -n "$sql" ] || { echo "offsite: no SQL dump in $dir" >&2; exit 1; }
name=${sql%.sql.gz}
# shellcheck disable=SC2086 # $acct is empty when there is no account service backup
tar -C "$dir" -cf - "$sql" $acct | age -r "$recipient" -o "$tmp/$name.tar.age"
rclone copyto "$tmp/$name.tar.age" "r2:$R2_BUCKET/$name.tar.age"
rclone delete --min-age 14d "r2:$R2_BUCKET"
echo "$(date -u +%FT%TZ) offsite: uploaded $name.tar.age ($(du -h "$tmp/$name.tar.age" | cut -f1)); bucket has $(rclone lsf "r2:$R2_BUCKET" | wc -l) objects"
