#!/bin/sh
# Managed disposable realm only; no arbitrary database addresses or credentials.
set -eu
umask 077
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
    cp "/opt/acore/etc/$component.conf.dist" "/state/config/$component.conf"
    # Replace keys rather than appending duplicate definitions.
    sed -i \
      -e "s|^LoginDatabaseInfo[[:space:]]*=.*|LoginDatabaseInfo = \"mysql;3306;acore;$password;acore_auth\"|" \
      -e "s|^CharacterDatabaseInfo[[:space:]]*=.*|CharacterDatabaseInfo = \"mysql;3306;acore;$password;acore_characters\"|" \
      -e "s|^WorldDatabaseInfo[[:space:]]*=.*|WorldDatabaseInfo = \"mysql;3306;acore;$password;acore_world\"|" \
      -e 's|^LogsDir[[:space:]]*=.*|LogsDir = "/state/logs"|' \
      -e 's|^TempDir[[:space:]]*=.*|TempDir = "/state/temp"|' \
      -e 's|^SourceDirectory[[:space:]]*=.*|SourceDirectory = "/azerothcore"|' \
      -e 's|^DataDir[[:space:]]*=.*|DataDir = "/server-data"|' \
      -e 's|^Console.Enable[[:space:]]*=.*|Console.Enable = 0|' \
      "/state/config/$component.conf"
}
configure dbimport
configure authserver
configure worldserver
cp /azerothcore/modules/mod-playerbots/conf/playerbots.conf.dist /opt/acore/etc/modules/playerbots.conf.dist
cp /opt/acore/etc/modules/playerbots.conf.dist /opt/acore/etc/modules/playerbots.conf
sed -i \
    -e "s|^PlayerbotsDatabaseInfo[[:space:]]*=.*|PlayerbotsDatabaseInfo = \"mysql;3306;acore;$password;acore_playerbots\"|" \
    -e "s|^Playerbots.RandomBotAutologin[[:space:]]*=.*|Playerbots.RandomBotAutologin = $bot_autologin|" \
    -e "s|^Playerbots.MinRandomBots[[:space:]]*=.*|Playerbots.MinRandomBots = $bot_count|" \
    -e "s|^Playerbots.MaxRandomBots[[:space:]]*=.*|Playerbots.MaxRandomBots = $bot_count|" \
    -e 's|^Playerbots.EnablePeriodicOnlineOffline[[:space:]]*=.*|Playerbots.EnablePeriodicOnlineOffline = 0|' \
    -e 's|^Playerbots.AddClassAccountPoolSize[[:space:]]*=.*|Playerbots.AddClassAccountPoolSize = 0|' \
    -e 's|^Playerbots.RandomBotGuildCount[[:space:]]*=.*|Playerbots.RandomBotGuildCount = 0|' \
    /opt/acore/etc/modules/playerbots.conf
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
