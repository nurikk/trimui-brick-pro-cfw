#!/bin/sh

trimui_root_fingerprint() {
    root=$(CDPATH= cd -- "$1" && pwd -P) || return 2
    printf '%s' "$root" | sha256sum | awk '{print substr($1, 1, 16)}'
}

trimui_checkout_fingerprint() {
    root=$(CDPATH= cd -- "$1" && pwd -P) || return 2
    find "$root" -type f \
        ! -path "$root/.git" \
        ! -path "$root/.git/*" \
        ! -path "$root/target/*" \
        ! -path "$root/.baseline-work/*" \
        -print | LC_ALL=C sort |
        while IFS= read -r path; do
            printf '%s\n' "${path#"$root"/}"
            sha256sum "$path" | awk '{print $1}'
        done | sha256sum | awk '{print $1}'
}

trimui_docker_namespace() {
    root=$1
    if [ "${TRIMUI_DOCKER_NAMESPACE+x}" = x ]; then
        case "$TRIMUI_DOCKER_NAMESPACE" in
        '' | [!a-z0-9]* | *[!a-z0-9-]* | *-)
            printf '%s\n' 'docker-worktree: TRIMUI_DOCKER_NAMESPACE must be 1-48 lowercase letters, digits, or hyphens, starting with a letter or digit' >&2
            return 2
            ;;
        esac
        [ "${#TRIMUI_DOCKER_NAMESPACE}" -le 48 ] || {
            printf '%s\n' 'docker-worktree: TRIMUI_DOCKER_NAMESPACE is too long (maximum 48 characters)' >&2
            return 2
        }
        printf '%s' "$TRIMUI_DOCKER_NAMESPACE"
        return 0
    fi
    printf 'wt-%s' "$(trimui_root_fingerprint "$root")"
}
