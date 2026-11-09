#!/usr/bin/env bash
# ci.sh -- the gate for this repository: one definition of what every host runs.
#
# The gate is a set of faces, and its verdict is the exit code:
#
#   deps face      scripts/check-deps.py   (dependency one-way law)
#   build face     cargo build --workspace --all-targets --locked
#   lib-test face  cargo test --workspace --lib --locked
#
# Three rules make the verdict mean something:
#
#   1. A face that was promised and cannot run is a red run. A gate that
#      reports a verdict it cannot trust is worse than no gate, so a
#      missing face stops the run red and says which piece is missing.
#   2. The faces are declared, not discovered: a face that is added to
#      this list and then deleted from the tree fails the run instead of
#      silently disappearing from it.
#   3. The gate reads the tree it lives in and nothing else: no credential,
#      no state outside the checkout. The same script therefore runs on a
#      CI runner, on a workstation and in a push hook, and the three answer
#      the same way. The one thing it does reach out for is the crates the
#      committed `Cargo.lock` names -- this repository is a Rust program,
#      so building it means fetching its dependencies. Those versions are
#      pinned in the lock file, so what arrives is what the tree says.
#
# The integration face (`cargo test --test integration`) is deliberately
# NOT in this list. It drives a real `mcc` daemon over a fixed port and
# reads a library stack from the developer's machine, so it answers a
# question about the host, not about this tree -- a runner without that
# host would report a red that says nothing about the code. It runs on the
# machine that has the daemon; the gate here answers "does the tree hold
# together".
#
# Host-agnostic on purpose: nothing here knows about a runner image, a
# token or a forge.
#
# Usage: scripts/ci.sh [--faces "deps build lib-test"]
set -u
cd "$(dirname "$0")/.."

FACES="deps build lib-test"
ALL_FACES="deps build lib-test"

# What each face runs, from the repository root.
face_command() {
    case "$1" in
        deps) echo "python3 scripts/check-deps.py ." ;;
        build) echo "cargo build --workspace --all-targets --locked" ;;
        lib-test) echo "cargo test --workspace --lib --locked" ;;
        *) echo "" ;;
    esac
}

while [ $# -gt 0 ]; do
    case "$1" in
        --faces) FACES="${2:-}"; shift 2 ;;
        --faces=*) FACES="${1#--faces=}"; shift ;;
        -h|--help)
            echo "usage: ci.sh [--faces \"$ALL_FACES\"]"
            exit 0
            ;;
        *) echo "ci.sh: unknown argument: $1" >&2; exit 2 ;;
    esac
done

note() { printf '%s\n' "$*"; }
error() { printf '::error::%s\n' "$*"; }

if ! command -v python3 >/dev/null 2>&1; then
    error "python3 is missing. Every face of this gate needs it; without it the run would report a verdict it cannot trust, so the run stops here."
    exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
    error "cargo is missing. This repository is a Rust workspace; without the toolchain the run would report a verdict it cannot trust, so the run stops here."
    exit 1
fi

failed=""
for face in $FACES; do
    command=$(face_command "$face")
    if [ -z "$command" ]; then
        error "unknown face: $face"
        exit 2
    fi
    script=${command#* }
    script=${script%% *}
    case "$face" in
        deps)
            if [ ! -f "$script" ]; then
                error "the $face face was promised but $script is not in the tree. A face that cannot run is a red run, not a green one with a silent gap."
                failed="$failed $face"
                continue
            fi
            ;;
    esac
    note "--- face: $face ($command)"
    if ! $command; then
        error "the $face face failed ($command)"
        failed="$failed $face"
    fi
done

if [ -n "$failed" ]; then
    note "ci.sh: FAILED (faces:$failed)"
    exit 1
fi

note "ci.sh: ok (faces: $FACES)"
