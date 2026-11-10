#!/usr/bin/env bash
# One-time host setup for the beta box (Ubuntu 24.04, run as root). Idempotent.
# Usable before the repo is on the box:  ssh root@box 'bash -s' < bootstrap-host.sh
# Packages and security updates, unattended-upgrades, Docker Engine + Compose plugin from
# Docker's apt repository, a 4 GB swapfile, key-only sshd, fail2ban for sshd.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
[ "$(id -u)" = 0 ] || { echo 'Run as root' >&2; exit 1; }
log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*"; }

log 'apt: updates and base packages'
apt-get update -q
apt-get -y -q -o Dpkg::Options::=--force-confold upgrade
apt-get install -y -q ca-certificates curl gnupg git rsync tmux jq bc xxd openssl unzip \
    unattended-upgrades fail2ban python3-systemd age rclone

log 'unattended-upgrades: daily security updates, no automatic reboot'
cat > /etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
EOF
cat > /etc/apt/apt.conf.d/52wrathplus-unattended <<'EOF'
Unattended-Upgrade::Automatic-Reboot "false";
Unattended-Upgrade::Remove-Unused-Kernel-Packages "true";
EOF
systemctl enable --now unattended-upgrades.service >/dev/null

log 'docker: engine + compose/buildx plugins from download.docker.com'
if ! command -v docker >/dev/null || ! docker compose version >/dev/null 2>&1; then
    install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
    chmod a+r /etc/apt/keyrings/docker.asc
    . /etc/os-release
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu ${UBUNTU_CODENAME:-$VERSION_CODENAME} stable" \
        > /etc/apt/sources.list.d/docker.list
    apt-get update -q
    apt-get install -y -q docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
fi
# Container log caps for anything not covered by the compose override.
if [ ! -f /etc/docker/daemon.json ]; then
    cat > /etc/docker/daemon.json <<'EOF'
{
  "log-driver": "json-file",
  "log-opts": { "max-size": "50m", "max-file": "5" }
}
EOF
    systemctl restart docker
fi
systemctl enable --now docker.service containerd.service >/dev/null

log 'swap: 4 GB /swapfile, swappiness 10'
if ! swapon --show=NAME --noheadings | grep -qx /swapfile; then
    [ -f /swapfile ] || fallocate -l 4G /swapfile
    chmod 600 /swapfile
    mkswap /swapfile >/dev/null
    swapon /swapfile
fi
grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
echo 'vm.swappiness = 10' > /etc/sysctl.d/90-wrathplus.conf
sysctl -q --system

log 'sshd: keys only'
# sshd takes the first value it reads; 10- sorts before cloud-init's 50- drop-in.
cat > /etc/ssh/sshd_config.d/10-wrathplus.conf <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
PubkeyAuthentication yes
PermitEmptyPasswords no
EOF
# Ubuntu 24.04 socket-activates sshd; -t needs its runtime dir, new connections read the config.
install -d -m 0755 /run/sshd
sshd -t
systemctl try-reload-or-restart ssh.service

log 'fail2ban: sshd jail (journald backend)'
cat > /etc/fail2ban/jail.d/wrathplus.local <<'EOF'
[sshd]
enabled = true
backend = systemd
maxretry = 5
findtime = 10m
bantime = 1h
EOF
systemctl enable fail2ban >/dev/null
systemctl restart fail2ban

log 'directories'
install -d -m 0755 /srv/wrathplus /srv/wrathplus/src /srv/wrathplus/server-data
install -d -m 0700 /srv/wrathplus/module-config /etc/wrathplus-beta /var/backups/wrathplus

log "done: $(docker --version); $(docker compose version)"
