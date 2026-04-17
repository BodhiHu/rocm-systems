#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
EMULATION_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd -- "${EMULATION_ROOT}/.." && pwd)"

log() {
    printf '[cleanup] %s\n' "$*"
}

find_mirage_ctl() {
    if [[ -n "${MIRAGE_CTL_BIN:-}" ]]; then
        printf '%s\n' "${MIRAGE_CTL_BIN}"
        return 0
    fi

    if command -v mirage-ctl >/dev/null 2>&1; then
        command -v mirage-ctl
        return 0
    fi

    local candidate
    for candidate in \
        "${REPO_ROOT}/target/debug/mirage_ctl" \
        "${REPO_ROOT}/target/release/mirage_ctl" \
        "${EMULATION_ROOT}/target/debug/mirage_ctl" \
        "${EMULATION_ROOT}/target/release/mirage_ctl"; do
        if [[ -x "${candidate}" ]]; then
            printf '%s\n' "${candidate}"
            return 0
        fi
    done

    return 1
}

resolve_runtime_dir() {
    local uid
    uid="$(id -u)"

    if [[ -n "${XDG_RUNTIME_DIR:-}" && "${XDG_RUNTIME_DIR}" = /* ]]; then
        printf '%s\n' "${XDG_RUNTIME_DIR}/mirage"
    elif [[ -d "/run/user/${uid}" ]]; then
        printf '%s\n' "/run/user/${uid}/mirage"
    else
        printf '%s\n' "/tmp/mirage-${uid}"
    fi
}

shutdown_sessions() {
    local socket_path="$1"
    local mirage_ctl
    local sessions
    local line
    local session_name

    if [[ ! -S "${socket_path}" ]]; then
        log "daemon socket not present at ${socket_path}; skipping session shutdown"
        return 0
    fi

    if ! mirage_ctl="$(find_mirage_ctl)"; then
        log "mirage_ctl not found; skipping session shutdown"
        return 0
    fi

    if ! sessions="$("${mirage_ctl}" --socket "${socket_path}" session list 2>/dev/null)"; then
        log "unable to list sessions through mirage_ctl; continuing with direct cleanup"
        return 0
    fi

    while IFS= read -r line; do
        [[ -z "${line}" || "${line}" == "no sessions found" ]] && continue
        session_name="$(printf '%s\n' "${line}" | awk -F'|' '{gsub(/^[[:space:]]+|[[:space:]]+$/, "", $1); print $1}')"
        [[ -z "${session_name}" ]] && continue

        log "shutting down session ${session_name}"
        if ! "${mirage_ctl}" --socket "${socket_path}" shutdown --name "${session_name}"; then
            log "shutdown failed for session ${session_name}; continuing"
        fi
    done <<< "${sessions}"
}

cleanup_containers() {
    local runtime
    local container_ids
    local container_id

    for runtime in docker podman; do
        if ! command -v "${runtime}" >/dev/null 2>&1; then
            continue
        fi

        if ! container_ids="$("${runtime}" ps -aq --filter name=mirage- 2>/dev/null)"; then
            log "unable to query ${runtime} containers"
            continue
        fi

        [[ -z "${container_ids}" ]] && continue
        log "removing Mirage containers with ${runtime}"

        while IFS= read -r container_id; do
            [[ -z "${container_id}" ]] && continue
            if ! "${runtime}" rm -f "${container_id}" >/dev/null 2>&1; then
                log "failed to remove container ${container_id} with ${runtime}"
            fi
        done <<< "${container_ids}"
    done
}

kill_processes() {
    local pids
    local pid
    local stubborn_pids=""

    pids="$(ps -eo pid=,args= | awk '$0 !~ /cleanup\.sh/ && ($0 ~ /mirage_daemon/ || $0 ~ /mirage_real/ || $0 ~ /mirage_remote/) { print $1 }')"

    [[ -z "${pids}" ]] && return 0
    log "terminating Mirage processes"

    while IFS= read -r pid; do
        [[ -z "${pid}" ]] && continue
        kill "${pid}" >/dev/null 2>&1 || true
    done <<< "${pids}"

    while IFS= read -r pid; do
        [[ -z "${pid}" ]] && continue
        if kill -0 "${pid}" >/dev/null 2>&1; then
            stubborn_pids+="${pid}"$'\n'
        fi
    done <<< "${pids}"

    [[ -z "${stubborn_pids}" ]] && return 0
    log "force killing remaining Mirage processes"

    while IFS= read -r pid; do
        [[ -z "${pid}" ]] && continue
        kill -9 "${pid}" >/dev/null 2>&1 || true
    done <<< "${stubborn_pids}"
}

cleanup_files() {
    local runtime_dir="$1"
    local socket_path="$2"

    rm -f -- "${socket_path}"

    if [[ -d "${runtime_dir}" ]]; then
        find "${runtime_dir}" -maxdepth 1 \( -type s -o -type f -o -type l \) -name 'mirage*' -exec rm -f -- {} +
        rmdir "${runtime_dir}" >/dev/null 2>&1 || true
    fi

    if [[ -d /tmp/mirage ]]; then
        find /tmp/mirage -maxdepth 1 \( -name 'emu-*.sock' -o -name 'topo-*' \) -exec rm -rf -- {} +
        rmdir /tmp/mirage >/dev/null 2>&1 || true
    fi

    find /tmp -maxdepth 1 -type d -name 'mirage-container-*' -exec rm -rf -- {} + 2>/dev/null || true
    find /tmp -maxdepth 1 -type s -name 'mirage-demo-*.sock' -exec rm -f -- {} + 2>/dev/null || true
}

main() {
    local runtime_dir
    local socket_path

    runtime_dir="$(resolve_runtime_dir)"
    socket_path="${MIRAGE_SOCKET:-${runtime_dir}/mirage.sock}"

    shutdown_sessions "${socket_path}"
    cleanup_containers
    kill_processes
    cleanup_files "${runtime_dir}" "${socket_path}"

    log "cleanup complete"
}

main "$@"