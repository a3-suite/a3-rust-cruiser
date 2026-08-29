#!/usr/bin/env bash

set -euo pipefail

# evidence_role: contract
# contract_id: ci.workflow-version-policy
# integration_id: runner-preinstalled-tool-identity

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
verifier="$repository_root/.ci/scripts/verify-ci-tools.sh"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-ci-tools-test.XXXXXX")"

cleanup() {
  python3 -c 'import pathlib, shutil, sys
path = pathlib.Path(sys.argv[1])
if path.is_symlink() or not path.is_dir():
    raise SystemExit("temporary root identity check failed")
shutil.rmtree(path)' "$test_root"
}
trap cleanup EXIT

write_mocks() {
  local mock_root="$1" rustup_version="$2" jq_version="$3" gh_version="$4"
  mkdir -p "$mock_root"
  printf '#!/usr/bin/env bash\nprintf '\''rustup %s (mock)\\n'\''\n' "$rustup_version" > "$mock_root/rustup"
  printf '#!/usr/bin/env bash\nprintf '\''%s\\n'\''\n' "$jq_version" > "$mock_root/jq"
  printf '#!/usr/bin/env bash\nprintf '\''gh version %s (mock)\\n'\''\n' "$gh_version" > "$mock_root/gh"
  chmod +x "$mock_root/rustup" "$mock_root/jq" "$mock_root/gh"
}

ubuntu_bin="$test_root/ubuntu-bin"
write_mocks "$ubuntu_bin" 1.29.0 jq-1.7 2.96.0
PATH="$ubuntu_bin:$PATH" bash "$verifier" ubuntu-24.04

macos_bin="$test_root/macos-bin"
write_mocks "$macos_bin" 1.29.0 jq-1.8.2 2.96.0
PATH="$macos_bin:$PATH" bash "$verifier" macos-15

mismatch_bin="$test_root/mismatch-bin"
write_mocks "$mismatch_bin" 1.28.2 jq-1.7 2.96.0
status=0
PATH="$mismatch_bin:$PATH" bash "$verifier" ubuntu-24.04 > /dev/null 2> "$test_root/error" || status=$?
[[ $status -ne 0 ]]
grep -F "rustup version mismatch" "$test_root/error" >/dev/null

jq_mismatch_bin="$test_root/jq-mismatch-bin"
write_mocks "$jq_mismatch_bin" 1.29.0 jq-1.6 2.96.0
status=0
PATH="$jq_mismatch_bin:$PATH" bash "$verifier" ubuntu-24.04 > /dev/null 2> "$test_root/error" || status=$?
[[ $status -ne 0 ]]
grep -F "jq version mismatch" "$test_root/error" >/dev/null

gh_mismatch_bin="$test_root/gh-mismatch-bin"
write_mocks "$gh_mismatch_bin" 1.29.0 jq-1.7 2.95.0
status=0
PATH="$gh_mismatch_bin:$PATH" bash "$verifier" ubuntu-24.04 > /dev/null 2> "$test_root/error" || status=$?
[[ $status -ne 0 ]]
grep -F "GitHub CLI version mismatch" "$test_root/error" >/dev/null

status=0
PATH="$ubuntu_bin:$PATH" bash "$verifier" unsupported-runner > /dev/null 2> "$test_root/error" || status=$?
[[ $status -ne 0 ]]
grep -F "unsupported runner label" "$test_root/error" >/dev/null

echo "CI tool version contract tests passed"
