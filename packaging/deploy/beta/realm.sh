#!/usr/bin/env bash
# WrathPlus beta realm operations on the box (run as root). See README.md.
#   realm.sh sources        fetch the pinned core, mod-playerbots and third-party modules
#   realm.sh build          sources + stage selected modules + build the amd64 image
#   realm.sh start          MySQL, bootstrap (dbimport), realm list row, auth + world; wait healthy
#   realm.sh stop           stop world and auth (MySQL and volumes kept)
#   realm.sh restart        stop + start
#   realm.sh update         git pull, build, backup, then stop + start on the new image
#   realm.sh backup         MySQL dump of the four schemas into $BACKUPS, keep $BACKUP_DAYS days
#   realm.sh status         services, health, memory, online bots
#   realm.sh account NAME   create a player account (no GM rights) over SOAP; password file under
#                           /etc/wrathplus-beta/accounts/NAME (0600), never printed
#   realm.sh soap CMD       one worldserver command over SOAP as the SOAPADMIN account
#   realm.sh install        install the daily backup timer (+ off-box copy, offsite.sh) and the
#                           wrathplus-realm and wrathplus-offsite commands
#   realm.sh account-service  Bun, services/account as a systemd unit (127.0.0.1:8787), its SOAP
#                           account (/etc/wrathplus/soap-account, 0600), Caddy with automatic TLS
#   realm.sh detach CMD     run CMD as a transient systemd unit, logging to $ROOT/logs
# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"
[ "$(id -u)" = 0 ] || die 'Run as root'

# --- sources ------------------------------------------------------------------------------------
fetch_pinned() { # dir url rev: a shallow clean checkout of exactly rev
    local dir=$1 url=$2 rev=$3
    if [ -d "$dir/.git" ] && [ "$(git -C "$dir" rev-parse HEAD 2>/dev/null)" = "$rev" ] \
        && [ -z "$(git -C "$dir" status --porcelain --untracked-files=no)" ]; then
        log "pinned $(basename "$dir") @ ${rev:0:12} present"
        return 0
    fi
    log "fetching $(basename "$dir") @ ${rev:0:12}"
    rm -rf "$dir"
    git init -q "$dir"
    git -C "$dir" remote add origin "$url"
    git -C "$dir" fetch -q --depth 1 origin "$rev"
    git -C "$dir" -c advice.detachedHead=false checkout -q --detach FETCH_HEAD
    [ "$(git -C "$dir" rev-parse HEAD)" = "$rev" ] || die "$dir is not at $rev"
}
cmd_sources() {
    umask 022
    mkdir -p "$SRC"
    local before=''
    [ -d "$CORE/.git" ] && before=$(git -C "$CORE" rev-parse HEAD 2>/dev/null || true)
    fetch_pinned "$CORE" "$CORE_URL" "$CORE_REV"
    # A re-fetched core lost its modules/ checkout.
    [ "$before" = "$CORE_REV" ] || rm -rf "$CORE/modules/mod-playerbots"
    fetch_pinned "$CORE/modules/mod-playerbots" "$PLAYERBOTS_URL" "$PLAYERBOTS_REV"
    fetch_pinned "$SRC/mod-multibot-bridge" "$MULTIBOT_URL" "$MULTIBOT_REV"
}

# Module-only build context, as the Manager stages it (crates/wrathplus-manager/src/modules/
# staging.rs): copies without .git; working .conf/.env files, links and special files refused.
stage_modules() {
    umask 022
    rm -rf "$CONTEXT"
    mkdir -p "$CONTEXT"
    local name
    rsync -a --chmod=Dgo+rx,Fgo+r --exclude .git "$CORE/modules/mod-playerbots/" "$CONTEXT/mod-playerbots/"
    rsync -a --chmod=Dgo+rx,Fgo+r --exclude .git "$SRC/mod-multibot-bridge/" "$CONTEXT/mod-multibot-bridge/"
    for name in $BUNDLED_MODULES; do
        rsync -a --chmod=Dgo+rx,Fgo+r --exclude .git "$RECIPE/modules/$name/" "$CONTEXT/$name/"
    done
    if [ -n "$(find "$CONTEXT" \( -type l -o ! -type f ! -type d -o -iname '*.conf' -o -name .env -o -name '.env.*' \) -print -quit)" ]; then
        die 'module context contains a link, special file or working .conf/.env file'
    fi
    log "staged modules: $(cd "$CONTEXT" && echo *)"
}

cmd_build() {
    cmd_sources
    stage_modules
    log "building $IMAGE with $BUILD_JOBS jobs (first build: hours)"
    compose --progress plain build playerbots-build
    local image
    image=$(docker image ls --quiet --no-trunc "$IMAGE")
    [ -n "$image" ] || die 'build finished but no image found'
    umask 022
    cat > "$ROOT/build-receipt.txt" <<EOF
built $(date -u +%FT%TZ)
image $IMAGE $image
repo $(git -C "$REPO" rev-parse HEAD)
core $CORE_REV
mod-playerbots $PLAYERBOTS_REV
mod-multibot-bridge $MULTIBOT_REV
bundled $BUNDLED_MODULES
EOF
    log "built $image"
}

# --- run ----------------------------------------------------------------------------------------
sql_literal() { case "$1" in *"'"*|*'\'*) die "unsafe value: $1";; esac; printf "'%s'" "$1"; }
cmd_start() {
    [ -n "$(docker image ls --quiet "$IMAGE")" ] || die "no $IMAGE image; run: realm.sh build"
    for d in dbc maps vmaps; do [ -d "$SERVER_DATA/$d" ] || die "server data lacks $d/"; done
    install -d -m 0700 "$MODULE_CONFIG"
    log 'starting MySQL'
    compose up -d --wait --wait-timeout 300 mysql
    log 'bootstrap: dbimport (core and module SQL)'
    compose run --rm bootstrap
    log "realm list: $REALM_NAME at $REALM_ADDRESS:$REALM_PORT"
    mysql_root <<SQL
UPDATE acore_auth.realmlist SET name=$(sql_literal "$REALM_NAME"),
  address=$(sql_literal "$REALM_ADDRESS"), localAddress=$(sql_literal "$REALM_ADDRESS"),
  localSubnetMask='255.255.255.255', port=$REALM_PORT, gamebuild=12340 WHERE id=1;
SQL
    log 'starting auth and world'
    compose up -d authserver worldserver
    wait_healthy 1200 || { compose ps; die 'auth/world not healthy after 20 minutes; see realm.sh status'; }
    log 'realm healthy'
}
cmd_stop() { compose stop worldserver authserver; }
cmd_restart() { cmd_stop; cmd_start; }

cmd_backup() {
    install -d -m 0700 "$BACKUPS"
    local file
    file="$BACKUPS/$PROJECT-$(date -u +%Y%m%d-%H%M%S).sql.gz"
    log "dumping to $file"
    compose exec -T mysql sh -c 'MYSQL_PWD=$(cat /run/instance-secrets/mysql-root) exec mysqldump -uroot \
        --single-transaction --quick --routines --events --triggers --set-gtid-purged=OFF \
        --databases acore_auth acore_characters acore_world acore_playerbots' \
        | gzip -6 > "$file.partial"
    gzip -t "$file.partial"
    mv "$file.partial" "$file"
    backup_account_service
    find "$BACKUPS" -maxdepth 1 \( -name "$PROJECT-*.sql.gz" -o -name "account-service-*.tar.gz" \) \
        -mtime +$((BACKUP_DAYS - 1)) -delete
    find "$BACKUPS" -maxdepth 1 -name "$PROJECT-*.partial" -mmin +600 -delete
    log "backup ok: $(du -h "$file" | cut -f1)"
}

# The account service's SQLite (consistent copy via VACUUM INTO, WAL-safe) and OAuth signing keys.
backup_account_service() {
    local data=/var/lib/wrathplus-account tmp file
    [ -f "$data/account.sqlite" ] && command -v bun >/dev/null || return 0
    tmp=$(mktemp -d)
    SRC_DB="$data/account.sqlite" DST_DB="$tmp/account.sqlite" bun -e \
        'const { Database } = require("bun:sqlite"); const db = new Database(process.env.SRC_DB); db.query("VACUUM INTO ?").run(process.env.DST_DB); db.close();'
    [ -f "$data/oauth-keys.json" ] && cp -p "$data/oauth-keys.json" "$tmp/"
    file="$BACKUPS/account-service-$(date -u +%Y%m%d-%H%M%S).tar.gz"
    tar -C "$tmp" -czf "$file" .
    rm -rf "$tmp"
    log "account service backup ok: $file"
}

cmd_update() {
    if [ "${1:-}" != --no-pull ]; then
        log 'updating repo'
        # lib.sh sets umask 077 for secrets; checkout files must stay readable (the image build
        # copies modules with their modes, and the server runs as an unprivileged user).
        (umask 022 && git -C "$REPO" pull --ff-only)
        # Re-exec so this run uses the pulled scripts and pins.
        exec "$BETA_DIR/realm.sh" update --no-pull
    fi
    cmd_build
    cmd_backup
    cmd_stop
    cmd_start
    # The account service runs from this checkout: refresh its dependencies and restart it.
    if systemctl -q is-enabled wrathplus-account.service 2>/dev/null; then cmd_account_service; fi
}

cmd_status() {
    compose ps --all
    echo
    docker stats --no-stream --format 'table {{.Name}}\t{{.MemUsage}}\t{{.MemPerc}}\t{{.CPUPerc}}'
    echo
    free -m
    echo
    printf 'online characters (bots + players): '
    mysql_root <<<'SELECT COUNT(*) FROM acore_characters.characters WHERE online = 1;' || true
}

# --- accounts over SOAP -------------------------------------------------------------------------
rev_hex() { local s=$1 out='' i; for ((i=${#s}-2; i>=0; i-=2)); do out+=${s:i:2}; done; printf '%s' "$out"; }
# AzerothCore SRP6 verifier, little-endian hex (as crates/wrathplus-srp password_verifier).
srp_verifier() { # user pass salt_hex
    local h1 h2 x v
    h1=$(printf '%s:%s' "$1" "$2" | tr '[:lower:]' '[:upper:]' | openssl dgst -sha1 -binary | xxd -p -c 256)
    h2=$(printf '%s%s' "$3" "$h1" | xxd -r -p | openssl dgst -sha1 -binary | xxd -p -c 256)
    x=$(rev_hex "$h2" | tr '[:lower:]' '[:upper:]')
    v=$(BC_LINE_LENGTH=0 bc <<BC
obase=16
ibase=16
n=894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7
x=$x
b=7
r=1
while (x>0) { if (x%2==1) r=(r*b)%n; b=(b*b)%n; x=x/2; }
r
BC
)
    rev_hex "$(printf '%064s' "$v" | tr ' ' 0)" | tr '[:upper:]' '[:lower:]'
}
random_password() { openssl rand -base64 48 | tr -dc 'A-HJ-NP-Z2-9' | head -c 16; }
# SOAP-only gmlevel-3 accounts. The account service treats the SOAP prefix as reserved, so no
# ATProto handle can be issued these names. Each is (re)written from its password file on every
# use, so a fresh DB or a lost file heals itself (as the Manager does with WPMANAGER).
SOAP_ADMIN=SOAPADMIN       # this script's commands; /etc/wrathplus-beta/soap-admin.secret
SOAP_SERVICE=SOAPACCOUNT   # the account service; USER:PASSWORD in $SOAP_SERVICE_FILE
SOAP_SERVICE_FILE=/etc/wrathplus/soap-account
gm_account() { # name password
    local salt verifier
    salt=$(openssl rand -hex 32)
    salt="${salt:0:62}$(printf '%02x' $(( 0x${salt:62:2} | 0x80 )))"
    verifier=$(srp_verifier "$1" "$2" "$salt")
    mysql_root <<SQL
INSERT INTO acore_auth.account (username, salt, verifier, email, reg_mail, expansion)
VALUES ('$1', X'$salt', X'$verifier', '', '', 2)
ON DUPLICATE KEY UPDATE salt = X'$salt', verifier = X'$verifier';
REPLACE INTO acore_auth.account_access (id, gmlevel, RealmID, comment)
SELECT id, 3, -1, 'WrathPlus beta SOAP only' FROM acore_auth.account WHERE username = '$1';
SQL
}
soap_admin() {
    local file=/etc/wrathplus-beta/soap-admin.secret
    install -d -m 0700 /etc/wrathplus-beta
    [ -s "$file" ] || random_password > "$file"
    chmod 600 "$file"
    gm_account "$SOAP_ADMIN" "$(cat "$file")"
}
xml_escape() { local s=${1//&/&amp;}; s=${s//</&lt;}; printf '%s' "${s//>/&gt;}"; }
xml_unescape() { local s=${1//&#xD;/}; s=${s//&lt;/<}; s=${s//&gt;/>}; s=${s//&quot;/\"}; s=${s//&apos;/\'}; printf '%s' "${s//&amp;/&}"; }
soap() { # one worldserver command; credentials and command go via stdin, not argv
    local response
    response=$(curl -sS --max-time 30 -K - --data-binary @<(printf '%s' \
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><SOAP-ENV:Envelope xmlns:SOAP-ENV=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:ns1=\"urn:AC\"><SOAP-ENV:Body><ns1:executeCommand><command>$(xml_escape "$1")</command></ns1:executeCommand></SOAP-ENV:Body></SOAP-ENV:Envelope>") \
        <<CURL
url = "http://127.0.0.1:17878/"
user = "$SOAP_ADMIN:$(cat /etc/wrathplus-beta/soap-admin.secret)"
header = "Content-Type: text/xml"
CURL
    )
    if [[ $response == *"<result>"* ]]; then
        response=${response#*<result>}; xml_unescape "${response%%</result>*}"; echo
    elif [[ $response == *"<result/>"* ]]; then
        :
    else
        response=${response#*<faultstring>}; printf 'SOAP fault: %s\n' "$(xml_unescape "${response%%</faultstring>*}")" >&2
        return 1
    fi
}
cmd_soap() { # realm.sh soap '.server info'
    [ -n "${1:-}" ] || die 'usage: realm.sh soap COMMAND'
    soap_admin
    soap "$1"
}
cmd_account() {
    local name=${1:-}
    [[ $name =~ ^[A-Za-z0-9]{3,16}$ ]] || die 'account name: 3-16 letters/digits'
    name=$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')
    local file=/etc/wrathplus-beta/accounts/$name
    install -d -m 0700 /etc/wrathplus-beta/accounts
    [ ! -e "$file" ] || die "$file exists; account already created"
    soap_admin
    random_password > "$file.new"
    soap ".account create $name $(cat "$file.new")" >/dev/null || { rm -f "$file.new"; die 'account create failed'; }
    mv "$file.new" "$file"
    log "account $name created (no GM rights); password in $file"
}

# --- account service (services/account): Bun, systemd unit, SOAP account, Caddy ----------------
ACCOUNT_DIR="$REPO/services/account"
cmd_account_service() {
    local arch=x64 tmp
    if ! command -v bun >/dev/null || [ "$(bun --version)" != "$BUN_VERSION" ]; then
        log "installing Bun $BUN_VERSION"
        tmp=$(mktemp -d)
        curl -fsSL -o "$tmp/bun-linux-$arch.zip" "https://github.com/oven-sh/bun/releases/download/bun-v$BUN_VERSION/bun-linux-$arch.zip"
        curl -fsSL -o "$tmp/SHASUMS256.txt" "https://github.com/oven-sh/bun/releases/download/bun-v$BUN_VERSION/SHASUMS256.txt"
        (cd "$tmp" && grep " bun-linux-$arch.zip\$" SHASUMS256.txt | sha256sum -c - >/dev/null) || die 'Bun checksum mismatch'
        (cd "$tmp" && unzip -q "bun-linux-$arch.zip")
        install -m 0755 "$tmp/bun-linux-$arch/bun" /usr/local/bin/bun
        rm -rf "$tmp"
    fi
    log "bun $(bun --version)"

    id wrathplus-account >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /var/lib/wrathplus-account --shell /usr/sbin/nologin wrathplus-account
    log 'service dependencies (bun install --frozen-lockfile --production)'
    (umask 022 && cd "$ACCOUNT_DIR" && bun install --frozen-lockfile --production >/dev/null)

    install -d -m 0750 -g wrathplus-account /etc/wrathplus
    if [ ! -f /etc/wrathplus/account.env ]; then
        install -m 0640 -g wrathplus-account "$ACCOUNT_DIR/account.env.example" /etc/wrathplus/account.env
        sed -i "s|^SOAP_CREDENTIALS_FILE=.*|SOAP_CREDENTIALS_FILE=$SOAP_SERVICE_FILE|" /etc/wrathplus/account.env
    fi
    # Its own gmlevel-3 SOAP account; USER:PASSWORD readable by the service user only.
    if [ ! -s "$SOAP_SERVICE_FILE" ]; then
        (umask 077 && printf '%s:%s' "$SOAP_SERVICE" "$(random_password)" > "$SOAP_SERVICE_FILE")
    fi
    chown wrathplus-account:wrathplus-account "$SOAP_SERVICE_FILE"
    chmod 600 "$SOAP_SERVICE_FILE"
    gm_account "$SOAP_SERVICE" "$(cut -d: -f2- "$SOAP_SERVICE_FILE")"

    install -m 0644 "$BETA_DIR/systemd/wrathplus-account.service" /etc/systemd/system/
    systemctl daemon-reload
    systemctl enable wrathplus-account.service >/dev/null
    systemctl restart wrathplus-account.service

    if ! command -v caddy >/dev/null; then
        log 'installing Caddy (stable apt repository)'
        curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/gpg.key | gpg --dearmor --yes -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
        curl -fsSL https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt > /etc/apt/sources.list.d/caddy-stable.list
        chmod 644 /usr/share/keyrings/caddy-stable-archive-keyring.gpg /etc/apt/sources.list.d/caddy-stable.list
        apt-get update -q
        DEBIAN_FRONTEND=noninteractive apt-get install -y -q caddy
    fi
    printf '# Managed by packaging/deploy/beta/realm.sh account-service\nimport %s\n' "$ACCOUNT_DIR/Caddyfile" > /etc/caddy/Caddyfile
    caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null 2>&1 || die "Caddyfile invalid"
    systemctl enable caddy >/dev/null
    systemctl reload-or-restart caddy
    log 'account service on 127.0.0.1:8787 behind Caddy (https://'"$REALM_ADDRESS"')'
}

# --- systemd ------------------------------------------------------------------------------------
cmd_install() {
    install -m 0644 "$BETA_DIR/systemd/wrathplus-backup.service" /etc/systemd/system/
    install -m 0644 "$BETA_DIR/systemd/wrathplus-backup.timer" /etc/systemd/system/
    install -d -m 0755 /etc/systemd/system/wrathplus-backup.service.d
    install -m 0644 "$BETA_DIR/systemd/wrathplus-backup.service.d/offsite.conf" \
        /etc/systemd/system/wrathplus-backup.service.d/
    ln -sf "$BETA_DIR/realm.sh" /usr/local/sbin/wrathplus-realm
    ln -sf "$BETA_DIR/offsite.sh" /usr/local/sbin/wrathplus-offsite
    systemctl daemon-reload
    systemctl enable --now wrathplus-backup.timer
    log 'installed: wrathplus-backup.timer (daily dump, then wrathplus-offsite); the realm restarts via Compose restart policies'
}

cmd_detach() {
    local command=${1:?command} logfile
    shift
    install -d -m 0755 "$ROOT/logs"
    logfile="$ROOT/logs/$command-$(date -u +%Y%m%d-%H%M%S).log"
    systemd-run --unit "wrathplus-$command" --collect --quiet \
        -p StandardOutput="append:$logfile" -p StandardError="append:$logfile" \
        "$BETA_DIR/realm.sh" "$command" "$@"
    log "started unit wrathplus-$command; log $logfile"
    log "poll: systemctl is-active wrathplus-$command; tail -f $logfile"
}

command=${1:-}
shift || true
case "$command" in
sources|build|start|stop|restart|update|backup|status|account|soap|install|detach) "cmd_$command" "$@" ;;
account-service) cmd_account_service "$@" ;;
*) sed -n '2,17p' "$0" >&2; exit 2 ;;
esac
