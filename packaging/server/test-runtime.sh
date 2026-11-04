#!/bin/sh
# Hermetic policy regression: no runtime startup, credentials, realm data or Docker.
set -eu
umask 077
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/runtime-policy.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
# Load only the pure functions from the shipped runtime (never its startup code).
sed -n '/^# BEGIN managed config helpers /,/^# END managed config helpers$/p' \
    "$here/runtime.sh" > "$work/helpers.sh"
. "$work/helpers.sh"
password=fixture_only
cat > "$work/core-policy" <<'EOF'
LoginDatabaseInfo = "mysql;3306;acore;fixture_only;acore_auth"
CharacterDatabaseInfo = "mysql;3306;acore;fixture_only;acore_characters"
WorldDatabaseInfo = "mysql;3306;acore;fixture_only;acore_world"
LogsDir = "/state/logs"
TempDir = "/state/temp"
SourceDirectory = "/azerothcore"
DataDir = "/server-data"
Console.Enable = 0
SOAP.Enabled = 1
SOAP.IP = "0.0.0.0"
SOAP.Port = 7878
EOF
check_policy() {
    helper=$1
    policy=$2
    # Similar names and comments are not owned definitions and must survive.
    printf '# Saved user settings\nUnowned = keep\nPlayerbotsXMaxRandomBots = keep\nConsoleXEnable = keep\n# Playerbots.MaxRandomBots = comment\n' > "$work/keep"
    cat "$work/keep" "$policy" > "$work/expected"
    for mode in missing indented duplicate unterminated; do
        cp "$work/keep" "$work/input"
        case "$mode" in
            missing) ;;
            indented) sed 's/^/  /; s/ = .*/ = unmanaged/' "$policy" >> "$work/input" ;;
            duplicate)
                sed 's/ = .*/ = unmanaged/' "$policy" >> "$work/input"
                sed 's/^/ 	/; s/ = .*/=unmanaged/' "$policy" >> "$work/input"
                ;;
            unterminated) printf 'Unterminated = keep' >> "$work/input" ;;
        esac
        "$helper" < "$work/input" > "$work/output"
        sed '/^$/d' "$work/output" > "$work/actual"
        if [ "$mode" = unterminated ]; then
            cat "$work/keep" > "$work/unterminated-expected"
            printf 'Unterminated = keep\n' >> "$work/unterminated-expected"
            cat "$policy" >> "$work/unterminated-expected"
            diff -u "$work/unterminated-expected" "$work/actual"
        else
            diff -u "$work/expected" "$work/actual"
        fi
        # Reapplication must still leave exactly one definition per owned key.
        "$helper" < "$work/output" | sed '/^$/d' > "$work/repeated"
        diff -u "$work/actual" "$work/repeated"
    done
}
check_policy managed_core_config "$work/core-policy"
printf 'PlayerbotsDatabaseInfo = "mysql;3306;acore;fixture_only;acore_playerbots"\n' > "$work/bot-policy"
check_policy managed_playerbots_config "$work/bot-policy"
# Population and turnover come from AC_PLAYERBOTS_* environment overrides (the Manager's
# server settings or server-defaults.env): saved values pass through untouched.
printf 'Playerbots.MinRandomBots = 7\nPlayerbots.MaxRandomBots = 9\nPlayerbots.RandomBotAutologin = 1\nPlayerbots.AddClassAccountPoolSize = 3\nPlayerbots.RandomBotGuildCount = 4\nPlayerbots.EnablePeriodicOnlineOffline = 1\n' > "$work/population"
managed_playerbots_config < "$work/population" | grep -v -e '^$' -e '^PlayerbotsDatabaseInfo ' > "$work/population-out"
diff -u "$work/population" "$work/population-out"
# Without the Manager, Compose falls back to server-defaults.env: every line one quoted
# AC_ override, nothing else.
if grep -v -e '^#' -e "^AC_[A-Z0-9_]*='[^']*'\$" "$here/server-defaults.env"; then
    echo 'server-defaults.env: unexpected line' >&2
    exit 1
fi
grep -q "^AC_PLAYERBOTS_MAX_RANDOM_BOTS='" "$here/server-defaults.env"
ahbot_account=7
ahbot_guid=1056
printf 'AuctionHouseBot.Account = 7\nAuctionHouseBot.GUID = 1056\n' > "$work/ahbot-policy"
check_policy managed_ahbot_config "$work/ahbot-policy"
printf 'runtime managed config policy tests passed\n'
