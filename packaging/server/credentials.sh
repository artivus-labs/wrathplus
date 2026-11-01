#!/bin/sh
# Instance-scoped generated credentials, never printed or passed in argv.
set -eu
umask 077
mkdir -p /secrets
for name in mysql-root mysql-app; do
    if [ ! -s "/secrets/$name" ]; then
        openssl rand -hex 32 > "/secrets/$name"
    fi
done
chown -R 10001:10001 /secrets
chmod 700 /secrets
chmod 600 /secrets/mysql-*
mkdir -p /init
password=$(cat /secrets/mysql-app)
cat > /init/01-managed.sql <<EOF
CREATE DATABASE IF NOT EXISTS acore_auth CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE IF NOT EXISTS acore_characters CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE IF NOT EXISTS acore_world CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE IF NOT EXISTS acore_playerbots CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER IF NOT EXISTS 'acore'@'%' IDENTIFIED BY '$password';
GRANT ALL PRIVILEGES ON acore_auth.* TO 'acore'@'%';
GRANT ALL PRIVILEGES ON acore_characters.* TO 'acore'@'%';
GRANT ALL PRIVILEGES ON acore_world.* TO 'acore'@'%';
GRANT ALL PRIVILEGES ON acore_playerbots.* TO 'acore'@'%';
EOF
# MySQL entrypoint drops to UID999 before processing initialization SQL.
chown -R 999:999 /init
chmod 700 /init
chmod 600 /init/01-managed.sql
echo 'Instance credentials ready (values withheld).'
