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
Playerbots.AddClassAccountPoolSize = $addclass_pool
Playerbots.RandomBotGuildCount = 0
EOF
}
# The auction house bot sells as the managed seller character (see managed_ahbot below).
managed_ahbot_config() {
    sed \
        -e '/^[[:space:]]*AuctionHouseBot\.Account[[:space:]]*=/d' \
        -e '/^[[:space:]]*AuctionHouseBot\.GUID[[:space:]]*=/d'
    cat <<EOF

AuctionHouseBot.Account = $ahbot_account
AuctionHouseBot.GUID = $ahbot_guid
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
# Accounts of ready-made bots for the addclass command (upstream's default, 50); 0 turns it off.
# Upstream creates their characters on the first start after this is raised.
addclass_pool=${MANAGED_ADDCLASS_POOL:-50}
case "$addclass_pool" in ''|*[!0-9]*) echo 'MANAGED_ADDCLASS_POOL must be a number' >&2; exit 2;; esac
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
# mod-ah-bot needs an account and a character to own its auctions. Unless a saved config names its
# own account, create an unloginable seller (random salt/verifier, no password exists) and its
# character once, before worldserver allocates guids, and point the bot at them.
managed_ahbot() {
    conf=/opt/acore/etc/modules/mod_ahbot.conf
    [ -f "$conf" ] || return 0
    if grep -Eq '^[[:space:]]*AuctionHouseBot\.Account[[:space:]]*=[[:space:]]*0*[1-9]' "$conf"; then
        echo 'STAGE: Auction house bot uses the account in its saved config'
        return 0
    fi
    sql acore_auth <<'SQL'
INSERT INTO account (username, salt, verifier, email, reg_mail, expansion)
SELECT 'AHBOT', RANDOM_BYTES(32), RANDOM_BYTES(32), '', '', 2 FROM DUAL
WHERE NOT EXISTS (SELECT 1 FROM account WHERE username = 'AHBOT');
SQL
    ahbot_account=$(sql -N -B acore_auth -e "SELECT id FROM account WHERE username = 'AHBOT'")
    case "$ahbot_account" in ''|*[!0-9]*) echo 'Auction house bot account missing; bot left disabled' >&2; return 0;; esac
    # A level-1 human warrior in Stormwind, as on the old dev realm. Skipped if a player took the name.
    sql acore_characters <<SQL
INSERT INTO characters (guid, account, name, race, class, gender, level, map, zone,
    position_x, position_y, position_z, orientation, taximask, innTriggerId, health)
SELECT COALESCE(MAX(guid), 0) + 1, $ahbot_account, 'Ahbot', 1, 1, 0, 1, 0, 1519,
    -8833.38, 628.628, 94.0066, 0.65, '', 0, 20 FROM characters
WHERE NOT EXISTS (SELECT 1 FROM characters WHERE account = $ahbot_account OR name = 'Ahbot');
SQL
    ahbot_guid=$(sql -N -B acore_characters -e "SELECT MIN(guid) FROM characters WHERE account = $ahbot_account")
    case "$ahbot_guid" in ''|NULL|*[!0-9]*) echo 'Auction house bot: the name Ahbot is taken; bot left disabled' >&2; return 0;; esac
    managed_ahbot_config < "$conf" > /state/config/mod_ahbot.conf
    if [ ! -f /state/module-overrides/mod_ahbot.conf ]; then
        # Upstream ships the seller off; a realm that chose this module wants a stocked market.
        sed -i 's/^[[:space:]]*AuctionHouseBot\.EnableSeller[[:space:]]*=.*/AuctionHouseBot.EnableSeller = 1/' \
            /state/config/mod_ahbot.conf
    fi
    cp /state/config/mod_ahbot.conf "$conf"
    echo "STAGE: Auction house bot sells as Ahbot (account $ahbot_account, guid $ahbot_guid)"
}
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
    if [ "$1" = worldserver ]; then
        managed_ahbot || echo 'Auction house bot setup failed; bot left disabled' >&2
    fi
    exec "$1" -c "/state/config/$1.conf"
    ;;
*) echo 'Expected bootstrap, authserver or worldserver' >&2; exit 2;;
esac
