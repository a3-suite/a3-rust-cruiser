#!/usr/bin/env bash

set -euo pipefail

# evidence_role: contract
# contract_id: git.release-flow
# integration_id: release-authority-handoff

test_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-release-authority-test.XXXXXX")"
repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
validator="$repository_root/.ci/scripts/validate-release-tag.sh"
asset_builder="$repository_root/.ci/scripts/build-release-asset.sh"
release_body_verifier="$repository_root/.ci/scripts/verify-release-body.sh"
mock_bin="$test_root/bin"

mkdir -p "$mock_bin"
# The generated mock expands its state variables when invoked, not while this fixture is constructed.
# shellcheck disable=SC2016
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'case "$*" in' \
  '  *git/ref/tags/*)' \
  '    case "${MOCK_GH_REMOTE_TAG_STATE:-current}" in' \
  '      current) jq -n --arg sha "$MOCK_GH_FROM_TAG_OBJECT_SHA" '\''{object:{sha:$sha,type:"tag"}}'\'' ;;' \
  '      mismatch) jq -n '\''{object:{sha:"1111111111111111111111111111111111111111",type:"tag"}}'\'' ;;' \
  '      invalid) printf '\''{}\n'\'' ;;' \
  '      unavailable) exit 1 ;;' \
  '      *) exit 2 ;;' \
  '    esac' \
  '    ;;' \
  '  *git/tags/*)' \
  '    case "${MOCK_GH_REMOTE_TAG_OBJECT_STATE:-current}" in' \
  '      current) jq -n --arg commit "$MOCK_GH_FROM_COMMIT" --arg tag "$MOCK_GH_FROM_TAG" '\''{tag:$tag,message:$tag,object:{sha:$commit,type:"commit"}}'\'' ;;' \
  '      tag-drift) jq -n --arg commit "$MOCK_GH_FROM_COMMIT" --arg tag "$MOCK_GH_FROM_TAG" '\''{tag:"changed-tag",message:$tag,object:{sha:$commit,type:"commit"}}'\'' ;;' \
  '      message-drift) jq -n --arg commit "$MOCK_GH_FROM_COMMIT" --arg tag "$MOCK_GH_FROM_TAG" '\''{tag:$tag,message:"changed message",object:{sha:$commit,type:"commit"}}'\'' ;;' \
  '      invalid) printf '\''{}\n'\'' ;;' \
  '      unavailable) exit 1 ;;' \
  '      *) exit 2 ;;' \
  '    esac' \
  '    ;;' \
  '  *tags?per_page=100*)' \
  '    case "${MOCK_GH_TAG_STATE:-current}" in' \
  '      current) printf '\''[[{"name":"v1.2.3"}]]\n'\'' ;;' \
  '      previous) printf '\''[[{"name":"v1.2.2"},{"name":"v1.2.3"}]]\n'\'' ;;' \
  '      invalid) printf '\''{}\n'\'' ;;' \
  '      unavailable) exit 1 ;;' \
  '      *) exit 2 ;;' \
  '    esac' \
  '    ;;' \
  '  *releases*)' \
  '    if [[ "$*" == *releases/latest* ]]; then' \
  '      case "${MOCK_GH_LATEST_RELEASE_STATE:-current}" in' \
  '        current) jq -n --arg tag "${MOCK_GH_LATEST_TAG:-v1.2.2}" '\''{draft:false,prerelease:false,tag_name:$tag}'\'' ;;' \
  '        invalid) printf '\''{}\n'\'' ;;' \
  '        unavailable) exit 1 ;;' \
  '        *) exit 2 ;;' \
  '      esac' \
  '    else' \
  '      case "${MOCK_GH_RELEASE_STATE:-empty}" in' \
  '        empty) printf '\''[[]]\n'\'' ;;' \
  '        published) printf '\''[[{"draft":false,"prerelease":false,"tag_name":"v0.9.0"}]]\n'\'' ;;' \
  '        invalid) printf '\''{}\n'\'' ;;' \
  '        unavailable) exit 1 ;;' \
  '        *) exit 2 ;;' \
  '      esac' \
  '    fi' \
  '    ;;' \
  '  *) exit 2 ;;' \
  'esac' \
  > "$mock_bin/gh"
chmod +x "$mock_bin/gh"

cleanup() {
  python3 -c 'import pathlib, shutil, sys
path = pathlib.Path(sys.argv[1])
if path.is_symlink() or not path.is_dir():
    raise SystemExit("temporary root identity check failed")
shutil.rmtree(path)' "$test_root"
}
trap cleanup EXIT

control_root="$test_root/control"
mkdir -p "$control_root"
git -C "$control_root" init -q
printf '[package]\nname = "release-fixture"\nversion = "1.2.2"\n' > "$control_root/Cargo.toml"
git -C "$control_root" add Cargo.toml
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid commit -qm initial
from_tag="v1.2.2"
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$from_tag" -m "$from_tag"
from_tag_object_sha="$(git -C "$control_root" rev-parse "refs/tags/$from_tag^{tag}")"
from_commit="$(git -C "$control_root" rev-parse "refs/tags/$from_tag^{commit}")"
printf '[package]\nname = "release-fixture"\nversion = "1.2.3"\n' > "$control_root/Cargo.toml"
git -C "$control_root" add Cargo.toml
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid commit -qm release
source_sha="$(git -C "$control_root" rev-parse HEAD)"
tag_name="v1.2.3"
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$tag_name" -m "$tag_name"
tag_object_sha="$(git -C "$control_root" rev-parse "refs/tags/$tag_name^{tag}")"

initial_control_root="$test_root/initial-control"
mkdir -p "$initial_control_root"
git -C "$initial_control_root" init -q
printf '[package]\nname = "release-fixture"\nversion = "1.2.3"\n' > "$initial_control_root/Cargo.toml"
git -C "$initial_control_root" add Cargo.toml
git -C "$initial_control_root" -c user.name=test -c user.email=test@example.invalid commit -qm initial
initial_source_sha="$(git -C "$initial_control_root" rev-parse HEAD)"
initial_tag_name="v1.2.3"
git -C "$initial_control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$initial_tag_name" -m "$initial_tag_name"
initial_tag_object_sha="$(git -C "$initial_control_root" rev-parse "refs/tags/$initial_tag_name^{tag}")"

control_sha="1111111111111111111111111111111111111111"
release_notes=$'## リリース\n - Version: 1.2.3\n - Tag: v1.2.3\n - Range: v1.2.2 -> v1.2.3\n\n## 重要な変更\n - Approved notes with `$` and `backticks`.\n\n## 主な変更内容\n - Release authority verification.\n'
initial_release_notes=$'## リリース\n - Version: 1.2.3\n - Tag: v1.2.3\n - Range: initial -> v1.2.3\n\n## 重要な変更\n - Initial approved notes.\n\n## 主な変更内容\n - Initial release authority verification.\n'

write_request() {
  local path="$1"
  local approval="$2"
  local requested_tag="$3"
  local requested_from="${4-$from_tag}"
  local requested_notes="${5-}"
  local requested_range_kind="${6-continuing}"
  local requested_source_sha="${7-$source_sha}"
  local range_start="$requested_from"
  if [[ "$requested_range_kind" == initial ]]; then range_start=initial; fi
  if [[ -z "$requested_notes" ]]; then
    printf -v requested_notes '## リリース\n - Version: 1.2.3\n - Tag: %s\n - Range: %s -> %s\n\n## 重要な変更\n - Approved notes.\n\n## 主な変更内容\n - Release authority verification.\n' "$requested_tag" "$range_start" "$requested_tag"
  fi

  jq -n \
    --arg event workflow_dispatch \
    --arg repository owner/repository \
    --arg run_id 123 \
    --arg control_sha "$control_sha" \
    --arg version 1.2.3 \
    --arg from "$requested_from" \
    --arg range_kind "$requested_range_kind" \
    --arg to "$requested_tag" \
    --arg tag "$requested_tag" \
    --arg target_commit "$requested_source_sha" \
    --arg target "GitHub Release" \
    --arg release_notes "$requested_notes" \
    --argjson approval "$approval" \
    '{event: $event, repository: $repository, run_id: $run_id, control_sha: $control_sha, version: $version, "range-kind": $range_kind, from: (if $from == "" then null else $from end), to: $to, tag: $tag, target_commit: $target_commit, target: $target, release_notes: $release_notes, approval: $approval}' \
    > "$path"
}

run_validator() {
  local request_file="$1"
  local output_dir="$2"
  local github_output="$3"
  local selected_control_root="${4:-$control_root}"

  GH_TOKEN=test-token \
  PATH="$mock_bin:$PATH" \
  EXPECTED_CONTROL_SHA="$control_sha" \
  EXPECTED_EVENT=workflow_dispatch \
  EXPECTED_REPOSITORY=owner/repository \
  EXPECTED_RUN_ID=123 \
  GITHUB_OUTPUT="$github_output" \
  MOCK_GH_FROM_COMMIT="${MOCK_GH_FROM_COMMIT:-$from_commit}" \
  MOCK_GH_FROM_TAG="${MOCK_GH_FROM_TAG:-$from_tag}" \
  MOCK_GH_FROM_TAG_OBJECT_SHA="${MOCK_GH_FROM_TAG_OBJECT_SHA:-$from_tag_object_sha}" \
    bash "$validator" "$request_file" "$selected_control_root" "$output_dir"
}

should_accept_approved_authority_without_branch_policy() {
  # Arrange
  local case_root="$test_root/approved"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output"

  # Assert
  jq -e \
    --arg source_sha "$source_sha" \
    --arg tag "$tag_name" \
    --arg tag_object_sha "$tag_object_sha" \
    --arg from_commit "$from_commit" \
    --arg from_tag_object_sha "$from_tag_object_sha" \
    '.contract_id == "git.release-flow" and .approval == true and .["range-kind"] == "continuing" and .from == "v1.2.2" and .from_commit == $from_commit and .from_tag_object_sha == $from_tag_object_sha and .source_sha == $source_sha and .tag == $tag and .tag_object_sha == $tag_object_sha' \
    "$case_root/authority/release-authority.json" >/dev/null
  grep -Fx "control_sha=$control_sha" "$case_root/github-output" >/dev/null
  printf '%s' "$release_notes" > "$case_root/expected-release-notes.md"
  cmp "$case_root/expected-release-notes.md" "$case_root/authority/release-notes.md"
}

should_accept_initial_release_authority() {
  # Arrange
  local case_root="$test_root/initial-approved"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root"

  # Assert
  jq -e \
    --arg source_sha "$initial_source_sha" \
    --arg tag "$initial_tag_name" \
    --arg tag_object_sha "$initial_tag_object_sha" \
    '.["range-kind"] == "initial" and .from == null and .from_commit == null and .from_tag_object_sha == null and .source_sha == $source_sha and .tag == $tag and .tag_object_sha == $tag_object_sha' \
    "$case_root/authority/release-authority.json" >/dev/null
  printf '%s' "$initial_release_notes" > "$case_root/expected-release-notes.md"
  cmp "$case_root/expected-release-notes.md" "$case_root/authority/release-notes.md"
}

should_reject_initial_release_with_non_null_from() {
  # Arrange
  local case_root="$test_root/initial-with-from"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" v1.2.2 "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release request has an invalid or unapproved schema" "$case_root/error" >/dev/null
}

should_reject_initial_release_when_release_state_is_unavailable() {
  # Arrange
  local case_root="$test_root/initial-unavailable"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  MOCK_GH_RELEASE_STATE=unavailable run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "GitHub release state is unavailable" "$case_root/error" >/dev/null
}

should_reject_initial_release_when_tag_state_is_unavailable() {
  # Arrange
  local case_root="$test_root/initial-tag-unavailable"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  MOCK_GH_TAG_STATE=unavailable run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "GitHub tag state is unavailable" "$case_root/error" >/dev/null
}

should_reject_initial_release_when_tag_state_is_invalid() {
  # Arrange
  local case_root="$test_root/initial-tag-invalid"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  MOCK_GH_TAG_STATE=invalid run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "GitHub tag state is invalid" "$case_root/error" >/dev/null
}

should_reject_initial_release_when_stable_release_exists() {
  # Arrange
  local case_root="$test_root/initial-published"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  MOCK_GH_RELEASE_STATE=published run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "initial release requires no published stable GitHub Releases" "$case_root/error" >/dev/null
}

should_reject_initial_release_when_release_state_is_invalid() {
  # Arrange
  local case_root="$test_root/initial-invalid-release-state"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  MOCK_GH_RELEASE_STATE=invalid run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "GitHub release state is invalid" "$case_root/error" >/dev/null
}

should_reject_initial_release_when_previous_tag_exists() {
  # Arrange
  local case_root="$test_root/initial-previous-tag"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$initial_tag_name" "" "$initial_release_notes" initial "$initial_source_sha"

  # Act
  local status=0
  MOCK_GH_TAG_STATE=previous run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$initial_control_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "initial release requires no previous release tags" "$case_root/error" >/dev/null
}

should_reject_continuing_release_with_null_from() {
  # Arrange
  local case_root="$test_root/continuing-null-from"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "" "" continuing

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release request has an invalid or unapproved schema" "$case_root/error" >/dev/null
}

should_reject_tag_that_does_not_match_version() {
  # Arrange
  local case_root="$test_root/tag-version-mismatch"
  local non_policy_tag="release-authority-test"
  mkdir -p "$case_root"
  git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$non_policy_tag" -m "$non_policy_tag"
  write_request "$case_root/request.json" true "$non_policy_tag"

  # Act
  local validator_status=0
  local builder_status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/validator-error" || validator_status=$?
  bash "$asset_builder" "$control_root" x86_64-unknown-linux-gnu "refs/tags/$non_policy_tag" "$source_sha" 1.2.3 "$case_root/asset" > /dev/null 2> "$case_root/builder-error" || builder_status=$?

  # Assert
  [[ $validator_status -ne 0 ]]
  [[ $builder_status -ne 0 ]]
  grep -Fx "release tag must match v{version}" "$case_root/validator-error" >/dev/null
  grep -Fx "tag and version mismatch" "$case_root/builder-error" >/dev/null
}

should_reject_lightweight_tag() {
  # Arrange
  local case_root="$test_root/lightweight"
  local lightweight_root="$case_root/control"
  local lightweight_tag="v1.2.3"
  mkdir -p "$case_root"
  git -C "$control_root" clone -q . "$lightweight_root"
  git -C "$lightweight_root" tag -d "$lightweight_tag" >/dev/null
  git -C "$lightweight_root" tag "$lightweight_tag"
  write_request "$case_root/request.json" true "$lightweight_tag" "$from_tag" "$release_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$lightweight_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release tag must be annotated" "$case_root/error" >/dev/null
}

should_reject_unapproved_release_notes() {
  # Arrange
  local case_root="$test_root/unapproved"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" false "$tag_name"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" >/dev/null 2>&1 || status=$?

  # Assert
  [[ $status -ne 0 ]]
}

should_reject_missing_release_range_start_tag() {
  # Arrange
  local case_root="$test_root/missing-from-tag"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" missing-release-tag

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" >/dev/null 2>&1 || status=$?

  # Assert
  [[ $status -ne 0 ]]
}

should_reject_release_range_start_at_target_commit() {
  # Arrange
  local case_root="$test_root/from-at-target"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$tag_name"

  # Act
  local status=0
  MOCK_GH_LATEST_TAG="$tag_name" run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release range start must be older than the target commit" "$case_root/error" >/dev/null
}

should_reject_non_ancestor_release_range_start() {
  # Arrange
  local case_root="$test_root/from-non-ancestor"
  local non_ancestor_tag=v1.2.1 empty_tree non_ancestor_commit
  mkdir -p "$case_root"
  empty_tree="$(git -C "$control_root" mktree < /dev/null)"
  non_ancestor_commit="$(printf 'unrelated release\n' | git -C "$control_root" -c user.name=test -c user.email=test@example.invalid commit-tree "$empty_tree")"
  git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$non_ancestor_tag" "$non_ancestor_commit" -m "$non_ancestor_tag"
  write_request "$case_root/request.json" true "$tag_name" "$non_ancestor_tag"

  # Act
  local status=0
  MOCK_GH_LATEST_TAG="$non_ancestor_tag" run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release range start must be an ancestor of the target commit" "$case_root/error" >/dev/null
}

should_reject_non_latest_release_range_start() {
  # Arrange
  local case_root="$test_root/from-not-latest"
  local old_tag=v1.2.0 old_commit
  mkdir -p "$case_root"
  old_commit="$(git -C "$control_root" rev-parse "${from_tag}^{commit}")"
  git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$old_tag" "$old_commit" -m "$old_tag"
  write_request "$case_root/request.json" true "$tag_name" "$old_tag"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release range start must match the previous stable GitHub Release tag" "$case_root/error" >/dev/null
}

should_reject_unavailable_previous_stable_release_state() {
  # Arrange
  local case_root="$test_root/latest-release-unavailable"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_LATEST_RELEASE_STATE=unavailable run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "previous stable GitHub Release state is unavailable" "$case_root/error" >/dev/null
}

should_reject_invalid_previous_stable_release_state() {
  # Arrange
  local case_root="$test_root/latest-release-invalid"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_LATEST_RELEASE_STATE=invalid run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release range start must match the previous stable GitHub Release tag" "$case_root/error" >/dev/null
}

should_reject_mismatched_remote_release_range_start_object() {
  # Arrange
  local case_root="$test_root/from-remote-object-mismatch"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_REMOTE_TAG_STATE=mismatch run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote tag object identity mismatch" "$case_root/error" >/dev/null
}

should_reject_unavailable_remote_release_range_start() {
  # Arrange
  local case_root="$test_root/from-remote-unavailable"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_REMOTE_TAG_STATE=unavailable run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote tag ref is unavailable" "$case_root/error" >/dev/null
}

should_reject_mismatched_remote_release_range_start_commit() {
  # Arrange
  local case_root="$test_root/from-remote-commit-mismatch"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_FROM_COMMIT=1111111111111111111111111111111111111111 run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote annotated tag identity mismatch" "$case_root/error" >/dev/null
}

should_reject_unavailable_remote_release_range_start_object() {
  # Arrange
  local case_root="$test_root/from-remote-object-unavailable"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_REMOTE_TAG_OBJECT_STATE=unavailable run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote tag object is unavailable" "$case_root/error" >/dev/null
}

should_reject_invalid_remote_release_range_start_object() {
  # Arrange
  local case_root="$test_root/from-remote-object-invalid"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_REMOTE_TAG_OBJECT_STATE=invalid run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote annotated tag identity mismatch" "$case_root/error" >/dev/null
}

should_reject_remote_release_range_start_tag_drift() {
  # Arrange
  local case_root="$test_root/from-remote-tag-drift"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_REMOTE_TAG_OBJECT_STATE=tag-drift run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote annotated tag identity mismatch" "$case_root/error" >/dev/null
}

should_reject_remote_release_range_start_message_drift() {
  # Arrange
  local case_root="$test_root/from-remote-message-drift"
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  MOCK_GH_REMOTE_TAG_OBJECT_STATE=message-drift run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "remote annotated tag identity mismatch" "$case_root/error" >/dev/null
}

should_reject_changed_release_tag_annotation() {
  # Arrange
  local case_root="$test_root/target-annotation" annotation_root="$test_root/target-annotation-control"
  mkdir -p "$case_root"
  git -C "$control_root" clone -q . "$annotation_root"
  git -C "$annotation_root" tag -d "$tag_name" >/dev/null
  git -C "$annotation_root" -c user.name=test -c user.email=test@example.invalid tag -a "$tag_name" -m "$tag_name" -m "release notes"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$annotation_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release tag annotation must exactly match the tag name" "$case_root/error" >/dev/null
}

should_reject_changed_release_range_start_annotation() {
  # Arrange
  local case_root="$test_root/from-annotation" annotation_root="$test_root/from-annotation-control"
  local from_commit
  mkdir -p "$case_root"
  git -C "$control_root" clone -q . "$annotation_root"
  from_commit="$(git -C "$annotation_root" rev-parse "${from_tag}^{commit}")"
  git -C "$annotation_root" tag -d "$from_tag" >/dev/null
  git -C "$annotation_root" -c user.name=test -c user.email=test@example.invalid tag -a "$from_tag" "$from_commit" -m "release notes"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$annotation_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release range start tag annotation must exactly match the tag name" "$case_root/error" >/dev/null
}

should_reject_release_tag_annotation_with_trailing_blank_line() {
  # Arrange
  local case_root="$test_root/target-annotation-blank-line" annotation_root="$test_root/target-annotation-blank-line-control"
  local message_file="$case_root/tag-message"
  mkdir -p "$case_root"
  git -C "$control_root" clone -q . "$annotation_root"
  git -C "$annotation_root" tag -d "$tag_name" >/dev/null
  printf '%s\n\n' "$tag_name" > "$message_file"
  git -C "$annotation_root" -c user.name=test -c user.email=test@example.invalid tag -a "$tag_name" --cleanup=verbatim -F "$message_file"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$release_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" "$annotation_root" > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx "release tag annotation must exactly match the tag name" "$case_root/error" >/dev/null
}

should_reject_release_notes_metadata_mismatch() {
  # Arrange
  local case_root="$test_root/metadata-mismatch"
  local mismatched_notes=$'## リリース\n - Version: 1.2.3\n - Tag: v1.2.3\n - Range: wrong-tag -> v1.2.3\n\n## 重要な変更\n - Approved notes.\n\n## 主な変更内容\n - Release authority verification.\n'
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$mismatched_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" >/dev/null 2>&1 || status=$?

  # Assert
  [[ $status -ne 0 ]]
}

should_reject_release_notes_version_mismatch() {
  # Arrange
  local case_root="$test_root/version-mismatch"
  local mismatched_notes=$'## リリース\n - Version: 9.9.9\n - Tag: v1.2.3\n - Range: v1.2.2 -> v1.2.3\n\n## 重要な変更\n - Approved notes.\n\n## 主な変更内容\n - Release authority verification.\n'
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$mismatched_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" >/dev/null 2>&1 || status=$?

  # Assert
  [[ $status -ne 0 ]]
}

should_reject_release_notes_tag_mismatch() {
  # Arrange
  local case_root="$test_root/tag-mismatch"
  local mismatched_notes=$'## リリース\n - Version: 1.2.3\n - Tag: wrong-tag\n - Range: v1.2.2 -> v1.2.3\n\n## 重要な変更\n - Approved notes.\n\n## 主な変更内容\n - Release authority verification.\n'
  mkdir -p "$case_root"
  write_request "$case_root/request.json" true "$tag_name" "$from_tag" "$mismatched_notes"

  # Act
  local status=0
  run_validator "$case_root/request.json" "$case_root/authority" "$case_root/github-output" >/dev/null 2>&1 || status=$?

  # Assert
  [[ $status -ne 0 ]]
}

should_accept_matching_published_release_notes() {
  # Arrange
  local case_root="$test_root/release-body-match"
  mkdir -p "$case_root"
  printf '%s' "$release_notes" > "$case_root/approved.md"

  # Act
  local status=0
  jq -n --arg body "$release_notes" '{body: $body}' | bash "$release_body_verifier" "$case_root/approved.md" || status=$?

  # Assert
  [[ $status -eq 0 ]]
}

should_reject_changed_published_release_notes() {
  # Arrange
  local case_root="$test_root/release-body-mismatch"
  mkdir -p "$case_root"
  printf '%s' "$release_notes" > "$case_root/approved.md"

  # Act
  local status=0
  jq -n --arg body "${release_notes}changed" '{body: $body}' | bash "$release_body_verifier" "$case_root/approved.md" >/dev/null 2>&1 || status=$?

  # Assert
  [[ $status -ne 0 ]]
}

should_accept_approved_authority_without_branch_policy
should_accept_initial_release_authority
should_reject_initial_release_with_non_null_from
should_reject_initial_release_when_release_state_is_unavailable
should_reject_initial_release_when_tag_state_is_unavailable
should_reject_initial_release_when_tag_state_is_invalid
should_reject_initial_release_when_stable_release_exists
should_reject_initial_release_when_release_state_is_invalid
should_reject_lightweight_tag
should_reject_unapproved_release_notes
should_reject_missing_release_range_start_tag
should_reject_release_range_start_at_target_commit
should_reject_non_ancestor_release_range_start
should_reject_non_latest_release_range_start
should_reject_unavailable_previous_stable_release_state
should_reject_invalid_previous_stable_release_state
should_reject_mismatched_remote_release_range_start_object
should_reject_unavailable_remote_release_range_start
should_reject_mismatched_remote_release_range_start_commit
should_reject_unavailable_remote_release_range_start_object
should_reject_invalid_remote_release_range_start_object
should_reject_remote_release_range_start_tag_drift
should_reject_remote_release_range_start_message_drift
should_reject_changed_release_tag_annotation
should_reject_changed_release_range_start_annotation
should_reject_release_tag_annotation_with_trailing_blank_line
should_reject_continuing_release_with_null_from
should_reject_tag_that_does_not_match_version
should_reject_release_notes_metadata_mismatch
should_reject_release_notes_version_mismatch
should_reject_release_notes_tag_mismatch
should_accept_matching_published_release_notes
should_reject_changed_published_release_notes
should_reject_initial_release_when_previous_tag_exists

bash "$repository_root/.ci/tests/publish-release-contract-test.sh"

echo "release authority contract tests passed"
