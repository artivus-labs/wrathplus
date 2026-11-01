#!/bin/sh
# Private runtime copy: provider settings are user-owned, managed DB credentials are not.
set -eu
umask 077
if [ "$(id -u)" = 0 ]; then
    mkdir -p /state
    [ -f /module-config/mod_llm_chatter.conf ] && [ ! -L /module-config/mod_llm_chatter.conf ] || {
        echo 'Save mod_llm_chatter.conf in Manager before starting the LLM bridge.' >&2; exit 2;
    }
    cp /module-config/mod_llm_chatter.conf /state/mod_llm_chatter.conf
    password=$(cat /run/instance-secrets/mysql-app)
    case "$password" in *[!0-9a-f]*|'') echo 'Invalid instance credential' >&2; exit 2;; esac
    # Remove keys first: upstream parsers need one authoritative definition.
    sed -i '/^[[:space:]]*LLMChatter\.Database\./d; /^[[:space:]]*LLMChatter\.Bridge\.Database\./d; /^[[:space:]]*LLMChatter\.HealthCheck\.LogPath[[:space:]]*=/d' /state/mod_llm_chatter.conf
    cat >> /state/mod_llm_chatter.conf <<EOF

LLMChatter.Database.Host = mysql
LLMChatter.Database.Port = 3306
LLMChatter.Database.User = acore
LLMChatter.Database.Password = $password
LLMChatter.Database.Name = acore_characters
LLMChatter.HealthCheck.LogPath = /state/healthcheck.log
EOF
    chown -R 10001:10001 /state
    chmod 700 /state
    chmod 600 /state/mod_llm_chatter.conf
    exec /usr/bin/setpriv --reuid=10001 --regid=10001 --clear-groups /bin/sh "$0"
fi
exec python /app/llm_chatter_bridge.py --config /state/mod_llm_chatter.conf
