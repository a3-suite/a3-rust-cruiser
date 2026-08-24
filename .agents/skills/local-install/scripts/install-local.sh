#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: .agents/skills/local-install/scripts/install-local.sh [--help]

Install a3-rust-cruiser from the current repository checkout and run its smoke test.
See .agents/skills/local-install/SKILL.md for the canonical procedure.

Options:
  --help, -h  Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_root="$(cd "$script_dir/.." && pwd)"
repo_root="$(cd "$skill_root/../../.." && pwd)"

command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
command -v rustc >/dev/null || { echo "rustc is required" >&2; exit 1; }

if [[ -n "${CARGO_INSTALL_ROOT:-}" ]]; then
  install_root="$CARGO_INSTALL_ROOT"
elif [[ -n "${CARGO_HOME:-}" ]]; then
  install_root="$CARGO_HOME"
else
  : "${HOME:?HOME is required when CARGO_INSTALL_ROOT and CARGO_HOME are unset}"
  install_root="$HOME/.cargo"
fi

binary_path="$install_root/bin/a3-rust-cruiser"

echo "[1/2] Installing a3-rust-cruiser from $repo_root"
(
  cd "$repo_root"
  cargo install --path . --locked --root "$install_root"
)

[[ -x "$binary_path" ]] || { echo "installed binary is not executable: $binary_path" >&2; exit 1; }

echo "[2/2] Running the installed CLI smoke test"
target_triple="$(cd "$repo_root" && rustc -vV | sed -n 's/^host: //p')"
[[ -n "$target_triple" ]] || { echo "failed to resolve the Rust host target" >&2; exit 1; }

smoke_output="$(
  "$binary_path" check "$repo_root/examples/minimal-project" \
    --policy "$repo_root/examples/minimal-project/boundary-policy.yaml" \
    --package minimal-project \
    --target bin:minimal-project \
    --target-triple "$target_triple" \
    --format text
)"
printf '%s\n' "$smoke_output"
grep -Fx 'decision: conformant' <<< "$smoke_output" >/dev/null || {
  echo "installed CLI smoke test did not report a conformant decision" >&2
  exit 1
}

printf 'Installed executable: %s\n' "$binary_path"
