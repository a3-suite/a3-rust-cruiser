#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: .agents/skills/commit-gate/scripts/run-commit-gate.sh [--help]

Validate and persistently install the staged snapshot.

Exit codes:
  0  commit allowed
  1  validation failed
  2  commit decision is indeterminate
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
temporary_root=""

cleanup() {
  local original_status=$?
  trap - EXIT

  if [[ -n "$temporary_root" && -e "$temporary_root" ]]; then
    if ! python3 -c 'import pathlib, shutil, sys
path = pathlib.Path(sys.argv[1])
if path.is_symlink() or not path.is_dir():
    raise SystemExit("temporary root identity check failed")
shutil.rmtree(path)' "$temporary_root"; then
      echo "Failed to remove temporary root: $temporary_root" >&2
      exit 2
    fi
  fi

  exit "$original_status"
}

trap cleanup EXIT

command -v git >/dev/null || { echo "git is required" >&2; exit 2; }
command -v bash >/dev/null || { echo "bash is required" >&2; exit 2; }
command -v python3 >/dev/null || { echo "python3 is required for safe cleanup" >&2; exit 2; }

resolved_git_root="$(git -C "$repo_root" rev-parse --show-toplevel 2>/dev/null)" || {
  echo "Failed to resolve the Git repository root" >&2
  exit 2
}
if [[ "$resolved_git_root" != "$repo_root" ]]; then
  echo "Repository root mismatch: expected $repo_root, got $resolved_git_root" >&2
  exit 2
fi

if git -C "$repo_root" diff --cached --quiet --exit-code; then
  echo "No staged changes; commit gate stopped" >&2
  exit 2
else
  staged_status=$?
  if [[ $staged_status -ne 1 ]]; then
    echo "Failed to inspect the staged snapshot" >&2
    exit 2
  fi
fi

temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-commit-gate.XXXXXX")" || {
  echo "Failed to create a temporary root" >&2
  exit 2
}
workspace_root="$temporary_root/workspace"
mkdir -p "$workspace_root"

if ! git -C "$repo_root" checkout-index --all --force --prefix="$workspace_root/"; then
  echo "Failed to materialize the staged snapshot" >&2
  exit 2
fi

local_install="$workspace_root/.agents/skills/local-install/scripts/install-local.sh"
if [[ ! -f "$local_install" ]]; then
  echo "The staged snapshot does not contain the local-install entrypoint" >&2
  exit 2
fi

echo "Validating and installing the staged snapshot"
if ! bash "$local_install"; then
  echo "Commit gate failed: local-install validation did not pass" >&2
  exit 1
fi

echo "Commit gate passed"
