#!/usr/bin/env bash

set -euo pipefail

# evidence_role: contract
# contract_id: git.release-flow
# integration_id: release-publication-boundary

test_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-publish-release-test.XXXXXX")"
repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
publisher="$repository_root/.ci/scripts/publish-github-release.sh"
mock_bin="$test_root/bin"

cleanup() {
  python3 -c 'import pathlib, shutil, sys
path = pathlib.Path(sys.argv[1])
if path.is_symlink() or not path.is_dir():
    raise SystemExit("temporary root identity check failed")
shutil.rmtree(path)' "$test_root"
}
trap cleanup EXIT

mkdir -p "$mock_bin"
cat > "$mock_bin/gh" <<'MOCK_GH'
#!/usr/bin/env bash
set -euo pipefail

method=GET
endpoint=
previous=
for argument in "$@"; do
  if [[ "$previous" == --method ]]; then method="$argument"; fi
  case "$argument" in
    repos/*|https://uploads.github.com/*) endpoint="$argument" ;;
  esac
  previous="$argument"
done

release_json() {
  local draft="$1"
  local title="${2:-$MOCK_TAG_NAME}"
  jq -n \
    --rawfile body "$MOCK_RELEASE_BODY_FILE" \
    --arg name "$title" \
    --arg tag "$MOCK_TAG_NAME" \
    --arg url "https://example.invalid/releases/$MOCK_TAG_NAME" \
    --argjson assets "$MOCK_ASSETS_JSON" \
    --argjson draft "$draft" \
    '{id: 77, tag_name: $tag, name: $name, draft: $draft, prerelease: false, body: $body, assets: $assets, html_url: $url}'
}

case "$endpoint" in
  repos/*/actions/artifacts/11)
    jq -n --arg digest "$MOCK_AUTHORITY_DIGEST" '{expired:false,name:"release-authority",digest:$digest,workflow_run:{id:123}}'
    ;;
  repos/*/actions/artifacts/12)
    jq -n --arg digest "$MOCK_LINUX_DIGEST" '{expired:false,name:"release-linux-x86_64",digest:$digest,workflow_run:{id:123}}'
    ;;
  repos/*/actions/artifacts/13)
    jq -n --arg digest "$MOCK_MACOS_DIGEST" '{expired:false,name:"release-macos-arm64",digest:$digest,workflow_run:{id:123}}'
    ;;
  repos/*/git/ref/tags/*)
    if [[ "$endpoint" == */v1.2.2 ]]; then
      if [[ "$MOCK_SCENARIO" == remote-object-before ]]; then
        jq -n '{object:{sha:"1111111111111111111111111111111111111111",type:"tag"}}'
      else
        jq -n --arg sha "$MOCK_FROM_TAG_OBJECT_SHA" '{object:{sha:$sha,type:"tag"}}'
      fi
    else
      jq -n --arg sha "$MOCK_TAG_OBJECT_SHA" '{object:{sha:$sha,type:"tag"}}'
    fi
    ;;
  repos/*/git/tags/*)
    if [[ "$endpoint" == */"$MOCK_FROM_TAG_OBJECT_SHA" ]]; then
      from_object_calls=0
      [[ ! -f "$MOCK_STATE_ROOT/from-object-calls" ]] || from_object_calls="$(cat "$MOCK_STATE_ROOT/from-object-calls")"
      ((from_object_calls += 1))
      printf '%s' "$from_object_calls" > "$MOCK_STATE_ROOT/from-object-calls"
      if [[ "$MOCK_SCENARIO" == remote-annotation-after && "$from_object_calls" -ge 2 ]]; then
        jq -n --arg sha "$MOCK_FROM_COMMIT" '{tag:"v1.2.2",message:"changed message",object:{sha:$sha,type:"commit"}}'
      else
        jq -n --arg sha "$MOCK_FROM_COMMIT" '{tag:"v1.2.2",message:"v1.2.2",object:{sha:$sha,type:"commit"}}'
      fi
    else
      jq -n --arg sha "$MOCK_SOURCE_SHA" '{tag:$ENV.MOCK_TAG_NAME,message:$ENV.MOCK_TAG_NAME,object:{sha:$sha,type:"commit"}}'
    fi
    ;;
  repos/*/tags\?per_page=100)
    if [[ "$MOCK_SCENARIO" == continuing ]]; then
      jq -n --arg tag "$MOCK_TAG_NAME" '[[{name:"v1.2.2"},{name:$tag}]]'
    else
      jq -n --arg tag "$MOCK_TAG_NAME" '[[{name:$tag}]]'
    fi
    ;;
  repos/*/releases/latest)
    latest_calls=0
    [[ ! -f "$MOCK_STATE_ROOT/latest-calls" ]] || latest_calls="$(cat "$MOCK_STATE_ROOT/latest-calls")"
    ((latest_calls += 1))
    printf '%s' "$latest_calls" > "$MOCK_STATE_ROOT/latest-calls"
    if [[ "$MOCK_SCENARIO" == continuing-race && "$latest_calls" -ge 2 ]]; then
      jq -n --arg tag "$MOCK_TAG_NAME" '{draft:false,prerelease:false,tag_name:$tag}'
    else
      printf '{"draft":false,"prerelease":false,"tag_name":"v1.2.2"}\n'
    fi
    ;;
  repos/*/releases\?per_page=100)
    release_calls=0
    [[ ! -f "$MOCK_STATE_ROOT/release-calls" ]] || release_calls="$(cat "$MOCK_STATE_ROOT/release-calls")"
    ((release_calls += 1))
    printf '%s' "$release_calls" > "$MOCK_STATE_ROOT/release-calls"
    if [[ "$MOCK_SCENARIO" == initial-race && "$release_calls" -ge 5 ]]; then
      jq -n --arg tag "$MOCK_TAG_NAME" '[[{id:77,tag_name:$tag,draft:true,prerelease:false},{id:76,tag_name:"v1.0.0",draft:false,prerelease:false}]]'
    elif [[ "$MOCK_SCENARIO" == continuing && -f "$MOCK_STATE_ROOT/published" ]]; then
      jq -n --argjson release "$(release_json false)" '[[$release,{id:76,tag_name:"v1.2.2",draft:false,prerelease:false}]]'
    elif [[ "$MOCK_SCENARIO" == continuing && -f "$MOCK_STATE_ROOT/created" ]]; then
      jq -n --argjson release "$(release_json true)" '[[$release,{id:76,tag_name:"v1.2.2",draft:false,prerelease:false}]]'
    elif [[ "$MOCK_SCENARIO" == continuing ]]; then
      printf '[[{"id":76,"tag_name":"v1.2.2","draft":false,"prerelease":false}]]\n'
    elif [[ -f "$MOCK_STATE_ROOT/published" ]]; then
      jq -n --argjson release "$(release_json false)" '[[$release]]'
    elif [[ -f "$MOCK_STATE_ROOT/created" ]]; then
      jq -n --argjson release "$(release_json true)" '[[$release]]'
    else
      printf '[[]]\n'
    fi
    ;;
  repos/*/releases)
    [[ "$method" == POST ]] || exit 2
    : > "$MOCK_STATE_ROOT/created"
    : > "$MOCK_STATE_ROOT/release-post"
    if [[ "$MOCK_SCENARIO" == title-post ]]; then release_json true changed-title; else release_json true; fi
    ;;
  https://uploads.github.com/*)
    [[ "$method" == POST ]] || exit 2
    printf '{}\n'
    ;;
  repos/*/releases/77)
    if [[ "$method" == PATCH ]]; then
      : > "$MOCK_STATE_ROOT/published"
      : > "$MOCK_STATE_ROOT/release-patch"
      if [[ "$MOCK_SCENARIO" == title-patch ]]; then release_json false changed-title; else release_json false "$MOCK_TAG_NAME"; fi
    elif [[ -f "$MOCK_STATE_ROOT/published" ]]; then
      if [[ "$MOCK_SCENARIO" == title-published ]]; then
        release_json false changed-title
      else
        release_json false
      fi
    else
      if [[ "$MOCK_SCENARIO" == title-draft ]]; then release_json true changed-title; else release_json true; fi
    fi
    ;;
  *)
    printf 'unexpected gh invocation: %s\n' "$*" >&2
    exit 2
    ;;
esac
MOCK_GH
chmod +x "$mock_bin/gh"

control_root="$test_root/control"
mkdir -p "$control_root"
git -C "$control_root" init -q
printf '[package]\nname = "release-fixture"\nversion = "1.2.2"\n' > "$control_root/Cargo.toml"
git -C "$control_root" add Cargo.toml
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid commit -qm initial
from_tag=v1.2.2
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$from_tag" -m "$from_tag"
from_tag_object_sha="$(git -C "$control_root" rev-parse "refs/tags/$from_tag^{tag}")"
from_commit="$(git -C "$control_root" rev-parse "refs/tags/$from_tag^{commit}")"
printf '[package]\nname = "release-fixture"\nversion = "1.2.3"\n' > "$control_root/Cargo.toml"
git -C "$control_root" add Cargo.toml
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid commit -qm release
source_sha="$(git -C "$control_root" rev-parse HEAD)"
tag_name=v1.2.3
tag_ref="refs/tags/$tag_name"
git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$tag_name" -m "$tag_name"
tag_object_sha="$(git -C "$control_root" rev-parse "$tag_ref^{tag}")"

write_handoff() {
  local handoff_root="$1" target="$2" asset_name asset_sha
  asset_name="a3-rust-cruiser-v1.2.3-$target.tar.gz"
  mkdir -p "$handoff_root"
  printf 'asset for %s\n' "$target" > "$handoff_root/$asset_name"
  asset_sha="$(sha256sum "$handoff_root/$asset_name" | awk '{print $1}')"
  jq -n \
    --arg asset_name "$asset_name" \
    --arg asset_sha "$asset_sha" \
    --arg source_sha "$source_sha" \
    --arg tag_ref "$tag_ref" \
    --arg target "$target" \
    '{schema_version:1,product:"a3-rust-cruiser",version:"1.2.3",tag_ref:$tag_ref,source_sha:$source_sha,target:$target,asset_name:$asset_name,asset_sha256:$asset_sha}' \
    > "$handoff_root/manifest.json"
}

write_authority() {
  local authority_root="$1" range_kind="$2" from_json="$3"
  local notes_sha
  mkdir -p "$authority_root"
  printf 'approved release notes\n' > "$authority_root/release-notes.md"
  notes_sha="$(sha256sum "$authority_root/release-notes.md" | awk '{print $1}')"
  jq -n \
    --arg control_sha "$source_sha" \
    --arg from_commit "$from_commit" \
    --arg from_tag_object_sha "$from_tag_object_sha" \
    --arg source_sha "$source_sha" \
    --arg tag "$tag_name" \
    --arg tag_object_sha "$tag_object_sha" \
    --arg tag_ref "$tag_ref" \
    --arg range_kind "$range_kind" \
    --argjson from "$from_json" \
    --arg notes_sha "$notes_sha" \
    '{schema_version:1,contract_id:"git.release-flow",approval:true,repository:"owner/repository",control_sha:$control_sha,source_sha:$source_sha,tag:$tag,tag_ref:$tag_ref,tag_object_sha:$tag_object_sha,version:"1.2.3",to:$tag,target:"GitHub Release",request_run_id:"123",release_notes_sha256:$notes_sha,"range-kind":$range_kind,from:$from,from_commit:(if $range_kind == "initial" then null else $from_commit end),from_tag_object_sha:(if $range_kind == "initial" then null else $from_tag_object_sha end)}' \
    > "$authority_root/release-authority.json"
}

run_publisher() {
  local case_root="$1" range_kind="$2" from_json="$3" scenario="$4"
  local authority_root="$case_root/authority" linux_root="$case_root/linux" macos_root="$case_root/macos"
  local authority_fingerprint linux_name macos_name linux_sha macos_sha checksums_sha assets_json
  mkdir -p "$case_root/state" "$case_root/runner-temp"
  write_authority "$authority_root" "$range_kind" "$from_json"
  if [[ "$scenario" == local-object-mismatch ]]; then
    jq '.from_tag_object_sha = "1111111111111111111111111111111111111111"' "$authority_root/release-authority.json" > "$authority_root/release-authority.tmp"
    mv "$authority_root/release-authority.tmp" "$authority_root/release-authority.json"
  elif [[ "$scenario" == local-commit-mismatch ]]; then
    jq '.from_commit = "1111111111111111111111111111111111111111"' "$authority_root/release-authority.json" > "$authority_root/release-authority.tmp"
    mv "$authority_root/release-authority.tmp" "$authority_root/release-authority.json"
  fi
  write_handoff "$linux_root" x86_64-unknown-linux-gnu
  write_handoff "$macos_root" aarch64-apple-darwin
  authority_fingerprint="$( { jq -cS . "$authority_root/release-authority.json"; cat "$authority_root/release-notes.md"; } | sha256sum | awk '{print $1}')"
  linux_name="$(jq -r .asset_name "$linux_root/manifest.json")"
  macos_name="$(jq -r .asset_name "$macos_root/manifest.json")"
  linux_sha="$(jq -r .asset_sha256 "$linux_root/manifest.json")"
  macos_sha="$(jq -r .asset_sha256 "$macos_root/manifest.json")"
  checksums_sha="$(printf '%s  %s\n%s  %s\n' "$macos_sha" "$macos_name" "$linux_sha" "$linux_name" | sort | sha256sum | awk '{print $1}')"
  assets_json="$(jq -n \
    --arg linux "$linux_name" --arg linux_digest "sha256:$linux_sha" \
    --arg macos "$macos_name" --arg macos_digest "sha256:$macos_sha" \
    --arg checksums_digest "sha256:$checksums_sha" \
    '[{name:$linux,digest:$linux_digest},{name:$macos,digest:$macos_digest},{name:"SHA256SUMS",digest:$checksums_digest}]')"

  GH_TOKEN=test-token \
  PATH="$mock_bin:$PATH" \
  EXPECTED_AUTHORITY_ARTIFACT_DIGEST=sha256:authority EXPECTED_AUTHORITY_ARTIFACT_ID=11 \
  EXPECTED_LINUX_ARTIFACT_DIGEST=sha256:linux EXPECTED_LINUX_ARTIFACT_ID=12 \
  EXPECTED_MACOS_ARTIFACT_DIGEST=sha256:macos EXPECTED_MACOS_ARTIFACT_ID=13 \
  EXPECTED_PLATFORMS='["x86_64-unknown-linux-gnu","aarch64-apple-darwin"]' \
  GITHUB_RUN_ID=123 RELEASE_AUTHORITY_FINGERPRINT="$authority_fingerprint" \
  RELEASE_CONTROL_SHA="$source_sha" RELEASE_REPOSITORY=owner/repository RELEASE_SOURCE_SHA="$source_sha" \
  RELEASE_TAG_NAME="$tag_name" RELEASE_TAG_OBJECT_SHA="$tag_object_sha" RELEASE_TAG_REF="$tag_ref" RELEASE_VERSION=1.2.3 \
  RUNNER_TEMP="$case_root/runner-temp" \
  MOCK_AUTHORITY_DIGEST=sha256:authority MOCK_LINUX_DIGEST=sha256:linux MOCK_MACOS_DIGEST=sha256:macos \
  MOCK_FROM_COMMIT="$from_commit" MOCK_FROM_TAG_OBJECT_SHA="$from_tag_object_sha" \
  MOCK_RELEASE_BODY_FILE="$authority_root/release-notes.md" MOCK_ASSETS_JSON="$assets_json" MOCK_SCENARIO="$scenario" \
  MOCK_SOURCE_SHA="$source_sha" MOCK_STATE_ROOT="$case_root/state" MOCK_TAG_NAME="$tag_name" MOCK_TAG_OBJECT_SHA="$tag_object_sha" \
    bash "$publisher" "$control_root" "$authority_root" "$linux_root" "$macos_root"
}

should_publish_initial_release() {
  # Arrange
  local case_root="$test_root/initial"
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" initial null initial > "$case_root/output"

  # Assert
  [[ -f "$case_root/state/release-patch" ]]
}

should_publish_continuing_release() {
  # Arrange
  local case_root="$test_root/continuing"
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' continuing > "$case_root/output"

  # Assert
  [[ -f "$case_root/state/release-patch" ]]
}

should_stop_initial_release_when_stable_release_appears() {
  # Arrange
  local case_root="$test_root/initial-race" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" initial null initial-race > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'initial release requires no published stable GitHub Releases' "$case_root/error" >/dev/null
  [[ -f "$case_root/state/release-post" ]]
  [[ ! -e "$case_root/state/release-patch" ]]
}

should_reject_invalid_authority_before_release_creation() {
  # Arrange
  local case_root="$test_root/invalid-authority" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" initial '"v1.2.2"' initial > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'release authority is invalid' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-post" ]]
}

should_reject_non_policy_tag_before_release_creation() {
  # Arrange
  local case_root="$test_root/non-policy-tag" status=0
  local saved_tag_name="$tag_name" saved_tag_ref="$tag_ref" saved_tag_object_sha="$tag_object_sha"
  mkdir -p "$case_root"
  tag_name=release-authority-test
  tag_ref="refs/tags/$tag_name"
  git -C "$control_root" -c user.name=test -c user.email=test@example.invalid tag -a "$tag_name" -m "$tag_name"
  tag_object_sha="$(git -C "$control_root" rev-parse "$tag_ref^{tag}")"

  # Act
  run_publisher "$case_root" initial null initial > /dev/null 2> "$case_root/error" || status=$?
  tag_name="$saved_tag_name"
  tag_ref="$saved_tag_ref"
  tag_object_sha="$saved_tag_object_sha"

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'release tag identity mismatch' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-post" ]]
}

should_reject_changed_release_title_in_post_response() {
  # Arrange
  local case_root="$test_root/title-post" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' title-post > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'created release is not the expected draft' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-patch" ]]
}

should_reject_changed_release_title_in_draft_readback() {
  # Arrange
  local case_root="$test_root/title-draft" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' title-draft > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'release changed before publication' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-patch" ]]
}

should_reject_changed_release_title_in_patch_response() {
  # Arrange
  local case_root="$test_root/title-patch" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' title-patch > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'release publication state mismatch' "$case_root/error" >/dev/null
  [[ -f "$case_root/state/release-patch" ]]
}

should_reject_changed_release_title_after_publication() {
  # Arrange
  local case_root="$test_root/title-published" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' title-published > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'published release state mismatch' "$case_root/error" >/dev/null
  [[ -f "$case_root/state/release-patch" ]]
}

should_stop_continuing_release_when_previous_release_changes() {
  # Arrange
  local case_root="$test_root/continuing-race" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' continuing-race > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'release range start is no longer the latest stable GitHub Release' "$case_root/error" >/dev/null
  [[ -f "$case_root/state/release-post" ]]
  [[ ! -e "$case_root/state/release-patch" ]]
}

should_stop_before_post_when_remote_from_object_differs() {
  # Arrange
  local case_root="$test_root/remote-object-before" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' remote-object-before > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'remote tag object identity mismatch' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-post" ]]
}

should_stop_before_patch_when_remote_from_annotation_changes() {
  # Arrange
  local case_root="$test_root/remote-annotation-after" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' remote-annotation-after > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'remote annotated tag identity mismatch' "$case_root/error" >/dev/null
  [[ -f "$case_root/state/release-post" ]]
  [[ ! -e "$case_root/state/release-patch" ]]
}

should_stop_before_post_when_local_from_object_differs() {
  # Arrange
  local case_root="$test_root/local-object-mismatch" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' local-object-mismatch > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'previous release tag object changed before publication' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-post" ]]
}

should_stop_before_post_when_local_from_commit_differs() {
  # Arrange
  local case_root="$test_root/local-commit-mismatch" status=0
  mkdir -p "$case_root"

  # Act
  run_publisher "$case_root" continuing '"v1.2.2"' local-commit-mismatch > /dev/null 2> "$case_root/error" || status=$?

  # Assert
  [[ $status -ne 0 ]]
  grep -Fx 'previous release tag commit changed before publication' "$case_root/error" >/dev/null
  [[ ! -e "$case_root/state/release-post" ]]
}

should_publish_initial_release
should_publish_continuing_release
should_stop_initial_release_when_stable_release_appears
should_reject_invalid_authority_before_release_creation
should_reject_non_policy_tag_before_release_creation
should_reject_changed_release_title_in_post_response
should_reject_changed_release_title_in_draft_readback
should_reject_changed_release_title_in_patch_response
should_reject_changed_release_title_after_publication
should_stop_continuing_release_when_previous_release_changes
should_stop_before_post_when_remote_from_object_differs
should_stop_before_patch_when_remote_from_annotation_changes
should_stop_before_post_when_local_from_object_differs
should_stop_before_post_when_local_from_commit_differs

echo "release publication contract tests passed"
