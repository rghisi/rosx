#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

SHELL_SRC=apps/shell/src/shell.rs

restore() {
    git checkout -- "$SHELL_SRC"
}

trap restore EXIT

probe_negative() {
    snippet=$1
    restore
    printf '\n%s\n' "$snippet" >> "$SHELL_SRC"
    set +e
    output=$(cargo xtask build 2>&1)
    status=$?
    set -e
    if [ "$status" -eq 0 ]; then
        echo "FAIL: build unexpectedly succeeded with injected snippet: $snippet" >&2
        exit 1
    fi
    case "$output" in
        *"error[E0308]"*)
            echo "OK: snippet rejected with error[E0308]: $snippet"
            ;;
        *)
            echo "FAIL: expected error[E0308] but build failed differently: $snippet" >&2
            printf '%s\n' "$output" >&2
            exit 1
            ;;
    esac
}

probe_negative 'fn _neg_kind_confusion(c: usrlib::handle::UserConnectionHandle) -> usrlib::handle::UserFutureHandle { c }'

probe_negative 'fn _neg_wrong_api(b: usrlib::handle::UserBindingHandle) { let _ = usrlib::ipc::ipc_send_value(b, 7); }'

restore
trap - EXIT

cargo xtask build

echo "OK: positive sanity build green after restore"
