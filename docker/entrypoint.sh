#!/bin/sh
# Start vislox in the container, configured from the environment
# (docker/README.md lists the variables).
set -eu

# A YAML single-quoted string.
yq() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/''/g")"; }

# git: who commits (Apply, Sync), and the config repository to clone.
git config --global user.name "${GIT_AUTHOR_NAME:-vislox}"
git config --global user.email "${GIT_AUTHOR_EMAIL:-vislox@localhost}"
git config --global --add safe.directory '*'
repo=/data/repo
if [ -n "${VISLOX_GIT_URL:-}" ] && [ ! -d "$repo/.git" ]; then
    echo "vislox: cloning the config repository into $repo"
    git clone ${VISLOX_GIT_BRANCH:+--branch "$VISLOX_GIT_BRANCH"} "$VISLOX_GIT_URL" "$repo"
fi

# lox's connection to the Miniserver (live values, Pull, Sync, Apply).
if [ -n "${LOX_HOST:-}" ]; then
    mkdir -p "$HOME/.lox"
    {
        echo "host: $(yq "$LOX_HOST")"
        echo "user: $(yq "${LOX_USER:-}")"
        echo "pass: $(yq "${LOX_PASS:-}")"
        echo "serial: $(yq "${LOX_SERIAL:-}")"
        if [ -n "${LOX_VERIFY_SSL:-}" ]; then echo "verify_ssl: $LOX_VERIFY_SSL"; fi
        if [ -n "${LOX_CONFIG_REPO:-}" ]; then echo "config_repo: $(yq "$LOX_CONFIG_REPO")"; fi
    } > "$HOME/.lox/config.yaml"
    chmod 600 "$HOME/.lox/config.yaml"
fi

set -- --listen "${VISLOX_LISTEN:-0.0.0.0:7878}"
secrets=$(mktemp -d)
if [ -n "${VISLOX_ADMIN_TOKEN:-}" ]; then
    printf '%s' "$VISLOX_ADMIN_TOKEN" > "$secrets/admin-token"
    set -- "$@" --admin-token-file "$secrets/admin-token"
fi
if [ -n "${VISLOX_AI_WEBHOOK:-}" ]; then
    printf '%s' "$VISLOX_AI_WEBHOOK" > "$secrets/ai-webhook"
    set -- "$@" --ai-webhook-file "$secrets/ai-webhook"
fi
unset VISLOX_ADMIN_TOKEN VISLOX_AI_WEBHOOK LOX_PASS

# Follow the repository's remote: changes pushed elsewhere show up in History.
# Only fast-forwards, and only while nothing is uncommitted.
if [ -n "${VISLOX_GIT_PULL_SECONDS:-}" ] && [ "$VISLOX_GIT_PULL_SECONDS" -gt 0 ] && [ -d "$repo/.git" ]; then
    (
        while sleep "$VISLOX_GIT_PULL_SECONDS"; do
            if [ -z "$(git -C "$repo" status --porcelain --untracked-files=no)" ]; then
                git -C "$repo" pull -q --ff-only || echo "vislox: git pull failed" >&2
            fi
        done
    ) &
fi

path=${VISLOX_PATH:-}
if [ -z "$path" ]; then
    if [ -d "$repo" ]; then path=$repo; else path=/data; fi
fi
# shellcheck disable=SC2086 # VISLOX_ARGS is a list of extra arguments
exec vislox "$path" "$@" ${VISLOX_ARGS:-}
