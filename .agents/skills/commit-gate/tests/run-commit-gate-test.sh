#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
gate_source="$(cd "$script_dir/../scripts" && pwd)/run-commit-gate.sh"
temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-commit-gate-test.XXXXXX")"

cleanup() {
  python3 -c 'import pathlib, shutil, sys
path = pathlib.Path(sys.argv[1])
if path.is_symlink() or not path.is_dir():
    raise SystemExit("temporary root identity check failed")
shutil.rmtree(path)' "$temporary_root"
}
trap cleanup EXIT

run_case() {
  local case_name="$1"
  local changed_path="$2"
  local expected_mode="$3"
  local original_path="${4:-}"
  local mutate_index="${5:-false}"
  local repo="$temporary_root/$case_name"
  local mode_log="$repo/mode.log"
  local mutate_repo=""
  local gate_status

  mkdir -p "$repo/.agents/skills/commit-gate/scripts"
  mkdir -p "$repo/.agents/skills/local-install/scripts"
  cp "$gate_source" "$repo/.agents/skills/commit-gate/scripts/run-commit-gate.sh"
  # Preserve these variables for expansion when the staged stub runs.
  # shellcheck disable=SC2016
  printf '%s\n' '#!/usr/bin/env bash' 'if [[ $# -eq 0 ]]; then' '  printf "install\n" > "$MODE_LOG"' 'else' '  printf "%s\n" "$1" > "$MODE_LOG"' 'fi' 'if [[ -n "${MUTATE_REPO:-}" ]]; then' '  printf "changed\n" > "$MUTATE_REPO/concurrent.txt"' '  git -C "$MUTATE_REPO" add concurrent.txt' 'fi' > "$repo/.agents/skills/local-install/scripts/install-local.sh"
  chmod +x "$repo/.agents/skills/local-install/scripts/install-local.sh"

  if [[ -n "$original_path" ]]; then
    mkdir -p "$(dirname "$repo/$original_path")"
    printf 'initial\n' > "$repo/$original_path"
  fi

  git -C "$repo" init -q
  git -C "$repo" add .
  git -C "$repo" -c user.name=test -c user.email=a commit -qm initial

  if [[ -n "$original_path" ]]; then
    mkdir -p "$(dirname "$repo/$changed_path")"
    git -C "$repo" mv "$original_path" "$changed_path"
  else
    mkdir -p "$(dirname "$repo/$changed_path")"
    printf 'changed\n' > "$repo/$changed_path"
    git -C "$repo" add "$changed_path"
  fi

  if [[ "$mutate_index" == "true" ]]; then
    mutate_repo="$repo"
  fi
  if MODE_LOG="$mode_log" MUTATE_REPO="$mutate_repo" bash "$repo/.agents/skills/commit-gate/scripts/run-commit-gate.sh" >/dev/null 2>&1; then
    gate_status=0
  else
    gate_status=$?
  fi
  if [[ "$mutate_index" == "true" ]]; then
    [[ $gate_status -eq 2 ]] || {
      echo "$case_name: expected gate status 2, got $gate_status" >&2
      return 1
    }
    return 0
  fi
  [[ $gate_status -eq 0 ]] || {
    echo "$case_name: expected gate status 0, got $gate_status" >&2
    return 1
  }
  [[ "$(<"$mode_log")" == "$expected_mode" ]] || {
    echo "$case_name: expected $expected_mode, got $(<"$mode_log")" >&2
    return 1
  }
}

run_case readme README.md --verify-installed
run_case security SECURITY.md --verify-installed
run_case example examples/minimal-project/src/main.rs --verify-installed
run_case source src/main.rs install
run_case source-rename docs/main.md install src/main.rs
run_case nested-markdown src/embedded.md install
run_case cargo Cargo.toml install
run_case installer .agents/skills/local-install/scripts/helper.sh install
run_case unknown assets/template.txt install
run_case concurrent-index README.md --verify-installed '' true

echo "commit gate tests passed"
