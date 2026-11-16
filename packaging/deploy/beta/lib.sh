# Shared helpers for the beta deployment scripts. Sourced, not executed.
# shellcheck shell=bash
set -euo pipefail
umask 077
# Bash 5.2 expands & in ${var//pat/rep} replacements; the XML helpers need it literal.
shopt -u patsub_replacement 2>/dev/null || true

BETA_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=beta.conf
. "$BETA_DIR/beta.conf"
if [ -f /etc/wrathplus-beta/beta.conf ]; then
    # shellcheck disable=SC1091
    . /etc/wrathplus-beta/beta.conf
fi
RECIPE="$REPO/packaging/server"

log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*"; }
die() { log "ERROR: $*" >&2; exit 1; }

# Server settings (AzerothCore AC_* overrides): a host file wins over the checked-in beta defaults.
server_env_file() {
    if [ -f /etc/wrathplus-beta/server.env ]; then
        echo /etc/wrathplus-beta/server.env
    else
        echo "$BETA_DIR/server.env"
    fi
}

# docker compose on the Manager's recipe (packaging/server/compose.yaml) plus the beta override,
# with the same environment the Manager passes (crates/wrathplus-manager/src/server/command.rs).
compose() {
    env -u COMPOSE_FILE -u COMPOSE_PROJECT_NAME -u COMPOSE_PROFILES -u COMPOSE_ENV_FILES \
        COMPOSE_DISABLE_ENV_FILE=1 \
        BUILDKIT_PROGRESS=plain \
        ACORE_SOURCE="$CORE" \
        MODULE_SOURCES="$CONTEXT" \
        MODULE_CONFIG="$MODULE_CONFIG" \
        SPIKE_DOCKERFILE="$RECIPE/Dockerfile" \
        SERVER_RECIPE="$RECIPE" \
        SERVER_DATA="$SERVER_DATA" \
        SERVER_ENV_FILE="$(server_env_file)" \
        CHATTER_SOURCE="$CHATTER_CONTEXT" \
        BETA_DIR="$BETA_DIR" \
        BETA_IMAGE="$IMAGE" \
        BETA_BUILD_JOBS="$BUILD_JOBS" \
        BETA_CHATTER_IMAGE="$CHATTER_IMAGE" \
        LLM_CHATTER_ENV_FILE="$LLM_CHATTER_ENV" \
        LLM_GUARD_SOURCE="$REPO/services/llm-guard" \
        LLM_GUARD_STATE="$LLM_GUARD_STATE" \
        LLM_GUARD_MONTHLY_USD="$LLM_GUARD_MONTHLY_USD" \
        docker compose --project-name "$PROJECT" --project-directory "$RECIPE" \
        -f "$RECIPE/compose.yaml" -f "$BETA_DIR/compose.beta.yaml" "$@"
}

# SQL as MySQL root inside the mysql container; the password never leaves the container.
mysql_root() {
    compose exec -T mysql sh -c 'MYSQL_PWD=$(cat /run/instance-secrets/mysql-root) exec mysql -uroot -N -B'
}

# Wait until mysql, authserver and worldserver are running and healthy (Manager's `ready`).
wait_healthy() {
    local deadline=$(( $(date +%s) + ${1:-900} ))
    while :; do
        local ok=1 service health
        for service in mysql authserver worldserver; do
            health=$(compose ps --format '{{.Health}}' "$service" 2>/dev/null || true)
            [ "$health" = healthy ] || ok=0
        done
        [ "$ok" = 1 ] && return 0
        [ "$(date +%s)" -lt "$deadline" ] || return 1
        sleep 10
    done
}

# mod_llm_chatter.conf from the module's .conf.dist and override files (stdout): each key an override
# sets replaces the shipped line in place (one definition per key; the C++ and Python parsers each
# need that); a later override wins; keys the .dist lacks are appended. Comments are kept.
render_chatter_config() { # dist override...
    local dist=$1
    shift
    awk -v dist="$dist" '
        function key(line) {
            if (line !~ /^[[:space:]]*[A-Za-z][A-Za-z0-9_.]*[[:space:]]*=/) return ""
            sub(/^[[:space:]]*/, "", line)
            sub(/[[:space:]]*=.*/, "", line)
            return line
        }
        FILENAME != dist {
            k = key($0)
            if (k == "") next
            if (!(k in value)) order[++n] = k
            value[k] = $0
            next
        }
        {
            k = key($0)
            if (k != "" && (k in value)) {
                if (!(k in used)) print value[k]
                used[k] = 1
                next
            }
            print
        }
        END {
            for (i = 1; i <= n; i++) {
                if (order[i] in used) continue
                if (!added++) print "\n# Added by the WrathPlus beta (packaging/deploy/beta/llm-chatter.conf)"
                print value[order[i]]
            }
        }
    ' "$@" "$dist"
}
