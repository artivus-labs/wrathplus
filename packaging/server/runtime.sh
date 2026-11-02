#!/bin/sh
# Managed disposable realm only; no arbitrary database addresses or credentials.
set -eu
umask 077
# BEGIN managed config helpers (pure stdin/stdout; also exercised hermetically).
# Delete every active Manager-owned definition, including indented duplicates, then
# append one authoritative policy. A leading newline also handles unterminated input.
managed_core_config() {
    sed \
        -e '/^[[:space:]]*LoginDatabaseInfo[[:space:]]*=/d' \
        -e '/^[[:space:]]*CharacterDatabaseInfo[[:space:]]*=/d' \
        -e '/^[[:space:]]*WorldDatabaseInfo[[:space:]]*=/d' \
        -e '/^[[:space:]]*LogsDir[[:space:]]*=/d' \
        -e '/^[[:space:]]*TempDir[[:space:]]*=/d' \
        -e '/^[[:space:]]*SourceDirectory[[:space:]]*=/d' \
        -e '/^[[:space:]]*DataDir[[:space:]]*=/d' \
        -e '/^[[:space:]]*Console\.Enable[[:space:]]*=/d' \
        -e '/^[[:space:]]*SOAP\.Enabled[[:space:]]*=/d' \
        -e '/^[[:space:]]*SOAP\.IP[[:space:]]*=/d' \
        -e '/^[[:space:]]*SOAP\.Port[[:space:]]*=/d'
    cat <<EOF

LoginDatabaseInfo = "mysql;3306;acore;$password;acore_auth"
CharacterDatabaseInfo = "mysql;3306;acore;$password;acore_characters"
WorldDatabaseInfo = "mysql;3306;acore;$password;acore_world"
LogsDir = "/state/logs"
TempDir = "/state/temp"
SourceDirectory = "/azerothcore"
DataDir = "/server-data"
Console.Enable = 0
SOAP.Enabled = 1
SOAP.IP = "0.0.0.0"
SOAP.Port = 7878
EOF
}
managed_playerbots_config() {
    sed \
        -e '/^[[:space:]]*PlayerbotsDatabaseInfo[[:space:]]*=/d' \
        -e '/^[[:space:]]*Playerbots\.RandomBotAutologin[[:space:]]*=/d' \
        -e '/^[[:space:]]*Playerbots\.MinRandomBots[[:space:]]*=/d' \
        -e '/^[[:space:]]*Playerbots\.MaxRandomBots[[:space:]]*=/d' \
        -e '/^[[:space:]]*Playerbots\.EnablePeriodicOnlineOffline[[:space:]]*=/d' \
        -e '/^[[:space:]]*Playerbots\.AddClassAccountPoolSize[[:space:]]*=/d' \
        -e '/^[[:space:]]*Playerbots\.RandomBotGuildCount[[:space:]]*=/d'
    cat <<EOF

PlayerbotsDatabaseInfo = "mysql;3306;acore;$password;acore_playerbots"
Playerbots.RandomBotAutologin = $bot_autologin
Playerbots.MinRandomBots = $bot_count
Playerbots.MaxRandomBots = $bot_count
Playerbots.EnablePeriodicOnlineOffline = 0
Playerbots.AddClassAccountPoolSize = 0
Playerbots.RandomBotGuildCount = 0
EOF
}
# END managed config helpers
# Host configs can contain API keys and are mode 0600. Copy privately as root,
# then drop privileges before loading server code or connecting to MySQL.
if [ "$(id -u)" = 0 ]; then
    command -v setpriv >/dev/null 2>&1 || { echo 'This server image lacks safe privilege dropping. Rebuild the server image before starting.' >&2; exit 2; }
    mkdir -p /state/module-overrides
    for override in /module-config/*.conf; do
        [ -f "$override" ] || continue
        [ ! -L "$override" ] || { echo 'Module config symlinks are not supported' >&2; exit 2; }
        cp "$override" "/state/module-overrides/$(basename "$override")"
    done
    chown -R 10001:10001 /state
    exec setpriv --reuid=10001 --regid=10001 --init-groups /bin/sh "$0" "$@"
fi
mkdir -p /opt/acore/etc/modules /state/logs /state/temp /state/config
# Module config lookup uses the compiled /opt/acore/etc path, not -c's parent.
# Deliberately bounded qualification population, never upstream's 500-bot default.
bot_count=${MANAGED_BOT_COUNT:-5}
case "$bot_count" in 0|1|2|3|4|5) ;; *) echo 'MANAGED_BOT_COUNT must be 0..5' >&2; exit 2;; esac
bot_autologin=0
if [ "$bot_count" -gt 0 ]; then bot_autologin=1; fi
password=$(cat /run/instance-secrets/mysql-app)
case "$password" in *[!0-9a-f]*|'') echo 'Invalid instance credential' >&2; exit 1;; esac
# Private client file keeps the password out of mysql argv and ordinary logs.
cat > /state/config/mysql.cnf <<EOF
[client]
host=mysql
port=3306
user=acore
password=$password
EOF
sql() { mysql --defaults-extra-file=/state/config/mysql.cnf "$@"; }
configure() {
    component=$1
    managed_core_config < "/opt/acore/etc/$component.conf.dist" > "/state/config/$component.conf"
}
configure dbimport
configure authserver
configure worldserver
# Installed defaults plus explicit persistent overrides; never write to the host mount.
for template in /opt/acore/etc/modules/*.conf.dist /azerothcore/modules/*/conf/*.conf.dist; do
    [ -f "$template" ] || continue
    name=$(basename "$template" .dist)
    [ -f "/opt/acore/etc/modules/$name" ] || cp "$template" "/opt/acore/etc/modules/$name"
done
for override in /state/module-overrides/*.conf; do
    [ -f "$override" ] || continue
    [ ! -L "$override" ] || { echo 'Module config symlinks are not supported' >&2; exit 2; }
    cp "$override" "/opt/acore/etc/modules/$(basename "$override")"
done
if [ -f /azerothcore/modules/mod-playerbots/conf/playerbots.conf.dist ]; then
    managed_playerbots_config < /opt/acore/etc/modules/playerbots.conf > /state/config/playerbots.conf
    cp /state/config/playerbots.conf /opt/acore/etc/modules/playerbots.conf
fi
case "${1:-}" in
bootstrap)
    echo 'STAGE: Initializing core schemas and applying upstream module SQL'
    # All four schemas are granted at first MySQL initialization, never a host DB.
    dbimport -c /state/config/dbimport.conf
    # Playerbots owns its fourth schema updater in worldserver, not dbimport.
    sql acore_auth <<'SQL'
UPDATE realmlist SET name='WrathPlus Local', address='127.0.0.1', localAddress='127.0.0.1', localSubnetMask='255.255.255.0', port=18085, gamebuild=12340 WHERE id=1;
SQL
    echo 'STAGE: Core bootstrap complete; Playerbots schema updates run during world startup'
    ;;
authserver|worldserver)
    echo "STAGE: Starting $1 (upstream database updates enabled)"
    exec "$1" -c "/state/config/$1.conf"
    ;;
*) echo 'Expected bootstrap, authserver or worldserver' >&2; exit 2;;
esac
