#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: .agents/skills/local-install/scripts/install-local.sh [--verify-installed] [--help]

Install a3-rust-cruiser from the current repository checkout and run its smoke test.
See .agents/skills/local-install/SKILL.md for the canonical procedure.

Options:
  --verify-installed  Verify the installed CLI without reinstalling it
  --help, -h           Show this help
EOF
}

mode="install"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --verify-installed)
      mode="verify-installed"
      ;;
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
  shift
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
      exit 1
    fi
  fi

  exit "$original_status"
}

trap cleanup EXIT

command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 1; }
command -v rustc >/dev/null || { echo "rustc is required" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 is required for safe cleanup" >&2; exit 1; }

if [[ -n "${CARGO_INSTALL_ROOT:-}" ]]; then
  install_root="$CARGO_INSTALL_ROOT"
elif [[ -n "${CARGO_HOME:-}" ]]; then
  install_root="$CARGO_HOME"
else
  : "${HOME:?HOME is required when CARGO_INSTALL_ROOT and CARGO_HOME are unset}"
  install_root="$HOME/.cargo"
fi

binary_path="$install_root/bin/a3-rust-cruiser"

preflight_command_path() {
  local path_preflight
  path_preflight="$(python3 -c 'import os, sys
expected = os.path.realpath(os.path.abspath(sys.argv[1]))
expected_binary = os.path.join(expected, "a3-rust-cruiser")
for entry in (entry or "." for entry in sys.argv[2].split(os.pathsep)):
    directory = os.path.realpath(os.path.abspath(entry))
    if directory == expected:
        raise SystemExit(0)
    candidate = os.path.join(directory, "a3-rust-cruiser")
    if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
        if os.path.exists(expected_binary) and os.path.samefile(candidate, expected_binary):
            raise SystemExit(0)
        print(f"a3-rust-cruiser is shadowed by {candidate}")
        raise SystemExit(1)
print(f"install root is not available on PATH: add {sys.argv[1]} to PATH")
raise SystemExit(1)' "$install_root/bin" "$PATH")" || {
    echo "$path_preflight" >&2
    return 1
  }
}

verify_installed_binary() {
  local expected_bin="$1"
  local resolved_binary
  preflight_command_path
  [[ -x "$expected_bin" ]] || {
    echo "installed binary is not executable: $expected_bin" >&2
    return 1
  }
  resolved_binary="$(command -v a3-rust-cruiser || true)"
  [[ -n "$resolved_binary" && "$resolved_binary" -ef "$expected_bin" ]] || {
    echo "installed command does not resolve to $expected_bin" >&2
    return 1
  }
}

run_smoke_test() {
  local executable="$1"
  local target_triple
  local smoke_output
  target_triple="$(cd "$repo_root" && rustc -vV | sed -n 's/^host: //p')"
  [[ -n "$target_triple" ]] || {
    echo "failed to resolve the Rust host target" >&2
    return 1
  }

  smoke_output="$(
    "$executable" check "$repo_root/examples/minimal-project" \
      --policy "$repo_root/examples/minimal-project/boundary-policy.yaml" \
      --package minimal-project \
      --target bin:minimal-project \
      --target-triple "$target_triple" \
      --format text
  )"
  printf '%s\n' "$smoke_output"
  grep -Fx 'decision: conformant' <<< "$smoke_output" >/dev/null || {
    echo "installed CLI smoke test did not report a conformant decision" >&2
    return 1
  }
}

if [[ "$mode" == "verify-installed" ]]; then
  echo "[1/1] Verifying the installed a3-rust-cruiser"
  verify_installed_binary "$binary_path"
  run_smoke_test "$binary_path"
  printf 'Reused executable: %s\n' "$binary_path"
  printf 'Installed command: a3-rust-cruiser\n'
  exit 0
fi

temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-local-install.XXXXXX")" || {
  echo "failed to create a temporary validation root" >&2
  exit 1
}
validation_root="$temporary_root/install"
validation_binary="$validation_root/bin/a3-rust-cruiser"

echo "[1/4] Validating a3-rust-cruiser from $repo_root"
(
  cd "$repo_root"
  cargo install --path . --locked --force --root "$validation_root"
)

[[ -x "$validation_binary" ]] || { echo "validation binary is not executable: $validation_binary" >&2; exit 1; }

echo "[2/4] Running the validation CLI smoke test"
run_smoke_test "$validation_binary"
preflight_command_path

echo "[3/4] Installing the validated CLI"
(
  cd "$repo_root"
  cargo install --path . --locked --force --root "$install_root"
)

[[ -x "$binary_path" ]] || { echo "installed binary is not executable: $binary_path" >&2; exit 1; }
hash -r
verify_installed_binary "$binary_path"

echo "[4/4] Running the installed CLI smoke test"
run_smoke_test "$binary_path"

printf 'Installed executable: %s\n' "$binary_path"
printf 'Installed command: a3-rust-cruiser\n'
