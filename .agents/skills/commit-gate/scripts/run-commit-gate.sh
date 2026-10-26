#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: .agents/skills/commit-gate/scripts/run-commit-gate.sh [--help]

Validate the staged snapshot and install it only when the CLI can change.

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

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
skill_root="$(cd "$script_dir/.." && pwd -P)"
repo_root="$(cd "$skill_root/../../.." && pwd -P)"
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

head_tree="$(git -C "$repo_root" rev-parse 'HEAD^{tree}' 2>/dev/null)" || {
  echo "Failed to resolve the HEAD tree" >&2
  exit 2
}
staged_tree="$(git -C "$repo_root" write-tree)" || {
  echo "Failed to capture the staged tree" >&2
  exit 2
}
if [[ "$staged_tree" == "$head_tree" ]]; then
  echo "No staged changes; commit gate stopped" >&2
  exit 2
fi

temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-commit-gate.XXXXXX")" || {
  echo "Failed to create a temporary root" >&2
  exit 2
}
workspace_root="$temporary_root/workspace"
snapshot_index="$temporary_root/staged.index"
changed_paths="$temporary_root/changed-paths"
mkdir -p "$workspace_root"

if ! GIT_INDEX_FILE="$snapshot_index" git -C "$repo_root" read-tree "$staged_tree"; then
  echo "Failed to prepare the staged tree index" >&2
  exit 2
fi
if ! GIT_INDEX_FILE="$snapshot_index" git -C "$repo_root" checkout-index --all --force --prefix="$workspace_root/"; then
  echo "Failed to materialize the staged snapshot" >&2
  exit 2
fi

local_install="$workspace_root/.agents/skills/local-install/scripts/install-local.sh"
if [[ ! -f "$local_install" ]]; then
  echo "The staged snapshot does not contain the local-install entrypoint" >&2
  exit 2
fi

install_required="false"
if ! git -C "$repo_root" diff-tree --no-commit-id --name-only --no-renames -z -r "$head_tree" "$staged_tree" > "$changed_paths"; then
  echo "Failed to classify the staged tree" >&2
  exit 2
fi
while IFS= read -r -d '' staged_path; do
  case "$staged_path" in
    README.md|CHANGELOG.md|CONTRIBUTING.md|SECURITY.md|LICENSE|docs/*|examples/*|tests/*|skills/*|sdd/*|.ci/*|.github/*|.gitignore|.gitattributes|.agents/skills/local-install/SKILL.md|.agents/skills/commit-gate/*)
      ;;
    *)
      install_required="true"
      break
      ;;
  esac
done < "$changed_paths"

verify_staged_tree_unchanged() {
  local current_tree
  current_tree="$(git -C "$repo_root" write-tree)" || {
    echo "Failed to recapture the staged tree" >&2
    return 1
  }
  [[ "$current_tree" == "$staged_tree" ]] || {
    echo "The staged tree changed while the commit gate was running" >&2
    return 1
  }
}

verify_staged_tree_unchanged || exit 2

if [[ "$install_required" == "true" ]]; then
  echo "Validating and installing the staged snapshot"
  if ! bash "$local_install"; then
    echo "Commit gate failed: local-install validation did not pass" >&2
    exit 1
  fi
else
  echo "CLI inputs are unchanged; verifying the existing installation"
  if ! bash "$local_install" --verify-installed; then
    echo "Commit gate failed: local-install validation did not pass" >&2
    exit 1
  fi
fi

verify_staged_tree_unchanged || exit 2

echo "Commit gate passed"
