#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
C4TUI_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
TUI_KIT_DIR="${TUI_KIT_DIR:-$(cd -- "${C4TUI_DIR}/../tui-kit" && pwd)}"
CARGO="${CARGO:-cargo}"

step() {
  printf '\n==> %s\n' "$*"
}

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

require_manifest() {
  local dir="$1"
  local name="$2"
  if [[ ! -f "${dir}/Cargo.toml" ]]; then
    printf 'error: %s Cargo.toml not found at %s\n' "${name}" "${dir}" >&2
    exit 1
  fi
}

require_manifest "${TUI_KIT_DIR}" "tui-kit"
require_manifest "${C4TUI_DIR}" "c4tui"

step "clean tui-kit"
run "${CARGO}" clean --manifest-path "${TUI_KIT_DIR}/Cargo.toml"

step "clean c4tui"
run "${CARGO}" clean --manifest-path "${C4TUI_DIR}/Cargo.toml"

step "build tui-kit"
run "${CARGO}" build --manifest-path "${TUI_KIT_DIR}/Cargo.toml"

step "install tui-kit"
printf 'tui-kit is a library crate, so Cargo has no binary target to install.\n'
printf 'c4tui consumes it through the local path dependency: %s\n' "${TUI_KIT_DIR}"

step "build c4tui"
run "${CARGO}" build --manifest-path "${C4TUI_DIR}/Cargo.toml"

step "install c4tui"
run "${CARGO}" install --path "${C4TUI_DIR}" --force

step "installed binary"
if command -v c4tui >/dev/null 2>&1; then
  command -v c4tui
else
  printf 'warning: c4tui was installed, but it is not on PATH\n' >&2
fi
