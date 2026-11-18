#!/usr/bin/env bash
# Runs on a Boat sandbox, as `user`. Idempotent; also used to upgrade a machine.
#   T3BOAT_SECRET=<watchdog-secret> provision.sh <t3-version> <sleep-after-secs> <snapshot-name> <mint-upstream-token:0|1> <name>
# Expects watchdog.mjs next to it. Prints one JSON line: {"environmentId", "upstreamToken"?}.
set -euo pipefail

version="$1" sleep_after="$2" snapshot="$3" mint_upstream="$4" name="$5"
secret="$T3BOAT_SECRET"
here="$(cd "$(dirname "$0")" && pwd)"
t3="$HOME/.local/bin/t3"
boat_dir="$HOME/.t3boat"
units="$HOME/.config/systemd/user"

# A new machine whose disk came from a template that ran T3 would share that server's identity
# (environment id, sessions, chats, links) with every other machine made from it.
if [ "$mint_upstream" = 1 ] && [ -e "$HOME/.t3/userdata/environment-id" ]; then
  systemctl --user stop t3boat-watchdog.service t3code.service 2>/dev/null || true
  rm -rf "$HOME/.t3/userdata" "$boat_dir"
fi
mkdir -p "$boat_dir" "$units" "$HOME/.t3/userdata"
chmod 700 "$boat_dir"

# T3 server pinned to the desktop app's version, as a systemd service that survives resumes.
# It is restarted only when something it runs on changed: a restart drops every T3 Code
# connection, and most upgrades only touch the watchdog.
restart_t3=0
if [ "$("$t3" --version 2>/dev/null || true)" != "t3 v$version" ]; then
  if ! curl -fsSL https://t3.codes/install.sh | T3CODE_VERSION="$version" sh >"$boat_dir/install.log" 2>&1; then
    echo "T3 install failed" >&2
    tail -n 40 "$boat_dir/install.log" >&2
    exit 1
  fi
  restart_t3=1
fi
settings="$HOME/.t3/userdata/settings.json"
before="$(cat "$settings" 2>/dev/null || true)"
node -e '
  const fs = require("fs"), f = process.argv[1];
  const s = fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, "utf8")) : {};
  s.linkedEnvironmentsEnabled = true;
  fs.writeFileSync(f, JSON.stringify(s, null, 2));
' "$settings"
[ "$before" = "$(cat "$settings")" ] || restart_t3=1
# The machine's name, as other devices show it (T3 Connect, linked environments). T3 reads it
# from here as it starts; without it, they show the sandbox's hostname.
machine_info="PRETTY_HOSTNAME=$name"
if [ "$(cat /etc/machine-info 2>/dev/null || true)" != "$machine_info" ]; then
  echo "$machine_info" | sudo tee /etc/machine-info >/dev/null
  restart_t3=1
fi
# Boat gives the sandbox's variables (the Boat environment's, gateway keys included, and BOAT_ID)
# only to shells: /run/ascii-secrets/env.sh, sourced by ~/.bashrc and written about half a minute
# after the user's services start, and again whenever Boat changes them (a resume, an environment
# upgrade). T3 is a service, so its agents would not see them. A oneshot waits for the file and
# imports its variables into the user manager before T3 starts; a path unit imports a changed file
# again and restarts T3 with it. They stay in memory and /run, never on the disk a snapshot keeps.
changed_units=""
put_unit() {
  if [ "$(cat "$1" 2>/dev/null || true)" != "$2" ]; then
    mkdir -p "$(dirname "$1")"
    printf '%s\n' "$2" >"$1"
    changed_units=1
  fi
}
put_unit "$boat_dir/boat-env.sh" "$(cat <<'SCRIPT'
#!/bin/bash
# boat-env.sh [refresh]. Names start with __t3boat_: the sourced file must not overwrite them.
set -u
__t3boat_file=/run/ascii-secrets/env.sh
__t3boat_run="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
__t3boat_mode="${1:-}"
for _ in $(seq 1 90); do [ -r "$__t3boat_file" ] && break; sleep 1; done
[ -r "$__t3boat_file" ] || exit 0
__t3boat_sum=$(sha256sum "$__t3boat_file" | cut -d' ' -f1)
if [ "$__t3boat_mode" = refresh ] && [ "$__t3boat_sum" = "$(cat "$__t3boat_run/t3boat-boat-env.sum" 2>/dev/null)" ]; then
  exit 0
fi
__t3boat_names=$(sed -nE 's/^export ([A-Za-z_][A-Za-z0-9_]*)=.*/\1/p' "$__t3boat_file" | sort -u)
. "$__t3boat_file" || exit 1
# As ~/.bashrc does: a Claude sign-in on the machine wins over the variables.
if [ -s "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/.credentials.json" ]; then
  unset ANTHROPIC_BASE_URL ANTHROPIC_AUTH_TOKEN CLAUDE_CODE_OAUTH_TOKEN
fi
__t3boat_set=$(for __t3boat_n in $__t3boat_names; do [ -n "${!__t3boat_n+x}" ] && echo "$__t3boat_n"; done)
if [ -s "$__t3boat_run/t3boat-boat-env.names" ]; then
  systemctl --user unset-environment $(cat "$__t3boat_run/t3boat-boat-env.names") || exit 1
fi
if [ -n "$__t3boat_set" ]; then
  systemctl --user import-environment $__t3boat_set || exit 1
fi
printf '%s\n' $__t3boat_set >"$__t3boat_run/t3boat-boat-env.names"
echo "$__t3boat_sum" >"$__t3boat_run/t3boat-boat-env.sum"
if [ "$__t3boat_mode" = refresh ]; then systemctl --user try-restart t3code.service; fi
SCRIPT
)"
put_unit "$units/t3boat-boat-env.service" "$(cat <<'UNIT'
[Unit]
Description=t3boat: the Boat environment's variables for T3 and its agents
Before=t3code.service

[Service]
Type=oneshot
RemainAfterExit=yes
TimeoutStartSec=120
ExecStart=/bin/bash %h/.t3boat/boat-env.sh

[Install]
WantedBy=default.target
UNIT
)"
put_unit "$units/t3boat-boat-env-refresh.service" "$(cat <<'UNIT'
[Unit]
Description=t3boat: T3 again with the Boat environment's changed variables
After=t3boat-boat-env.service

[Service]
Type=oneshot
ExecStart=/bin/bash %h/.t3boat/boat-env.sh refresh
UNIT
)"
put_unit "$units/t3boat-boat-env.path" "$(cat <<'UNIT'
[Unit]
Description=t3boat: watches the Boat environment's variables

[Path]
PathChanged=/run/ascii-secrets/env.sh
Unit=t3boat-boat-env-refresh.service

[Install]
WantedBy=default.target
UNIT
)"
put_unit "$units/t3code.service.d/t3boat-boat-env.conf" "$(cat <<'UNIT'
[Unit]
Wants=t3boat-boat-env.service
After=t3boat-boat-env.service
UNIT
)"
systemctl --user daemon-reload
systemctl --user enable t3boat-boat-env.service t3boat-boat-env.path >/dev/null 2>&1
systemctl --user start t3boat-boat-env.path
if [ -n "$changed_units" ] || ! systemctl --user is-active --quiet t3boat-boat-env.service; then
  systemctl --user restart t3boat-boat-env.service
  restart_t3=1
fi

"$t3" service install --allow-downgrade >/dev/null
systemctl --user daemon-reload
systemctl --user is-active --quiet t3code.service || restart_t3=1
[ "$restart_t3" = 0 ] || systemctl --user restart t3code.service

ready=0
for _ in $(seq 1 120); do
  if curl -sf -o /dev/null http://127.0.0.1:3773/.well-known/t3/environment; then ready=1; break; fi
  sleep 0.5
done
if [ "$ready" != 1 ]; then
  echo "T3 server did not start" >&2
  tail -n 40 "$HOME/.t3/userdata/logs/boot-service.log" >&2 || true
  exit 1
fi

# Watchdog: its own read-only T3 token, kept across upgrades.
config="$boat_dir/config.json"
watchdog_token="$(node -e 'try{process.stdout.write(require(process.argv[1]).t3Token||"")}catch{}' "$config")"
if [ -z "$watchdog_token" ]; then
  watchdog_token="$("$t3" auth session issue --token-only --ttl 3650d --label t3boat-watchdog --scope orchestration:read)"
fi
SECRET="$secret" TOKEN="$watchdog_token" SLEEP="$sleep_after" SNAPSHOT="$snapshot" node -e '
  const e = process.env;
  require("fs").writeFileSync(process.argv[1], JSON.stringify({
    secret: e.SECRET, t3Token: e.TOKEN, sleepAfterSecs: Number(e.SLEEP), snapshot: e.SNAPSHOT
  }, null, 2), { mode: 0o600 });
' "$config"
install -m 600 "$here/watchdog.mjs" "$boat_dir/watchdog.mjs"
cat >"$units/t3boat-watchdog.service" <<'UNIT'
[Unit]
Description=t3boat watchdog: stops this sandbox when idle
After=t3code.service

[Service]
ExecStart=/bin/bash -lc 'exec node --no-warnings %h/.t3boat/watchdog.mjs'
Restart=always
RestartSec=5
StandardOutput=append:%h/.t3boat/watchdog.log
StandardError=append:%h/.t3boat/watchdog.log

[Install]
WantedBy=default.target
UNIT
systemctl --user daemon-reload
systemctl --user enable t3boat-watchdog.service >/dev/null 2>&1
systemctl --user restart t3boat-watchdog.service

upstream_token=""
if [ "$mint_upstream" = 1 ]; then
  upstream_token="$("$t3" auth session issue --token-only --ttl 3650d --label t3boat)"
fi
environment_id="$(curl -sf http://127.0.0.1:3773/.well-known/t3/environment | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>process.stdout.write(JSON.parse(s).environmentId))')"
ENV_ID="$environment_id" TOK="$upstream_token" node -e '
  const o = { environmentId: process.env.ENV_ID };
  if (process.env.TOK) o.upstreamToken = process.env.TOK;
  console.log(JSON.stringify(o));
'
