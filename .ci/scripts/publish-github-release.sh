#!/usr/bin/env bash

set -euo pipefail

control_root="${1:?trusted control checkout is required}"
authority_handoff="${2:?release authority directory is required}"
linux_handoff="${3:?Linux handoff directory is required}"
macos_handoff="${4:?macOS handoff directory is required}"
script_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
release_assets_contract="$script_root/../release-assets.json"
required=(EXPECTED_AUTHORITY_ARTIFACT_DIGEST EXPECTED_AUTHORITY_ARTIFACT_ID EXPECTED_LINUX_ARTIFACT_DIGEST EXPECTED_LINUX_ARTIFACT_ID EXPECTED_MACOS_ARTIFACT_DIGEST EXPECTED_MACOS_ARTIFACT_ID EXPECTED_PLATFORMS GITHUB_RUN_ID GH_TOKEN RELEASE_AUTHORITY_FINGERPRINT RELEASE_CONTROL_SHA RELEASE_REPOSITORY RELEASE_SOURCE_SHA RELEASE_TAG_NAME RELEASE_TAG_OBJECT_SHA RELEASE_TAG_REF RELEASE_VERSION RUNNER_TEMP)
for name in "${required[@]}"; do [[ -n "${!name:-}" ]] || { echo "$name is required" >&2; exit 1; }; done

[[ "$RELEASE_TAG_REF" == "refs/tags/$RELEASE_TAG_NAME" && "$RELEASE_TAG_NAME" == "v$RELEASE_VERSION" ]] || { echo "release tag identity mismatch" >&2; exit 1; }
[[ "$EXPECTED_AUTHORITY_ARTIFACT_ID" =~ ^[0-9]+$ && "$EXPECTED_LINUX_ARTIFACT_ID" =~ ^[0-9]+$ && "$EXPECTED_MACOS_ARTIFACT_ID" =~ ^[0-9]+$ && "$GITHUB_RUN_ID" =~ ^[0-9]+$ ]] || { echo "artifact and run ids must be numeric" >&2; exit 1; }
[[ "$RELEASE_AUTHORITY_FINGERPRINT" =~ ^[0-9a-f]{64}$ ]] || { echo "invalid release authority fingerprint" >&2; exit 1; }
[[ "$RELEASE_CONTROL_SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid release control SHA" >&2; exit 1; }
[[ "$RELEASE_TAG_OBJECT_SHA" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid tag object SHA" >&2; exit 1; }
[[ "$(git -C "$control_root" rev-parse HEAD)" == "$RELEASE_CONTROL_SHA" ]] || { echo "release control checkout identity mismatch" >&2; exit 1; }
[[ "$(git -C "$control_root" cat-file -t "$RELEASE_TAG_REF")" == tag ]] || { echo "release tag must remain annotated" >&2; exit 1; }
[[ "$(git -C "$control_root" rev-parse --verify "${RELEASE_TAG_REF}^{tag}")" == "$RELEASE_TAG_OBJECT_SHA" ]] || { echo "release tag object changed before publication" >&2; exit 1; }
[[ "$(git -C "$control_root" rev-parse --verify "${RELEASE_TAG_REF}^{commit}")" == "$RELEASE_SOURCE_SHA" ]] || { echo "release tag changed before publication" >&2; exit 1; }

verify_live_tag() {
  local object_json object_sha object_type depth=0
  object_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/git/ref/tags/$RELEASE_TAG_NAME")"
  object_sha="$(jq -r '.object.sha' <<< "$object_json")"
  object_type="$(jq -r '.object.type' <<< "$object_json")"
  [[ "$object_type" == tag && "$object_sha" == "$RELEASE_TAG_OBJECT_SHA" ]] || { echo "live release tag object identity mismatch" >&2; exit 1; }
  while [[ "$object_type" == tag ]]; do
    ((depth += 1))
    [[ "$depth" -le 10 ]] || { echo "annotated tag chain is too deep" >&2; exit 1; }
    object_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/git/tags/$object_sha")"
    object_sha="$(jq -r '.object.sha' <<< "$object_json")"
    object_type="$(jq -r '.object.type' <<< "$object_json")"
  done
  [[ "$object_type" == commit && "$object_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "release tag does not resolve to a commit" >&2; exit 1; }
  [[ "$object_sha" == "$RELEASE_SOURCE_SHA" ]] || { echo "live release tag does not match the verified source SHA" >&2; exit 1; }
}

verify_artifact() {
  local artifact_id="$1" expected_digest="$2" expected_name="$3" metadata
  metadata="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/actions/artifacts/$artifact_id")"
  jq -e --arg digest "$expected_digest" --arg name "$expected_name" --argjson run_id "$GITHUB_RUN_ID" '.expired == false and .name == $name and .digest == $digest and .workflow_run.id == $run_id' <<< "$metadata" >/dev/null || { echo "artifact identity or digest mismatch: $artifact_id" >&2; exit 1; }
}
verify_artifact "$EXPECTED_AUTHORITY_ARTIFACT_ID" "$EXPECTED_AUTHORITY_ARTIFACT_DIGEST" release-authority
verify_artifact "$EXPECTED_LINUX_ARTIFACT_ID" "$EXPECTED_LINUX_ARTIFACT_DIGEST" release-linux-x86_64
verify_artifact "$EXPECTED_MACOS_ARTIFACT_ID" "$EXPECTED_MACOS_ARTIFACT_DIGEST" release-macos-arm64

authority_file="$authority_handoff/release-authority.json"
release_notes_file="$authority_handoff/release-notes.md"
[[ -d "$authority_handoff" && ! -L "$authority_handoff" ]] || { echo "invalid release authority directory" >&2; exit 1; }
[[ "$(find "$authority_handoff" -mindepth 1 -maxdepth 1 | wc -l | tr -d ' ')" == 2 ]] || { echo "release authority must contain exactly two entries" >&2; exit 1; }
[[ -f "$authority_file" && ! -L "$authority_file" && -f "$release_notes_file" && ! -L "$release_notes_file" ]] || { echo "release authority files are missing" >&2; exit 1; }
jq -e \
  --arg repository "$RELEASE_REPOSITORY" \
  --arg control_sha "$RELEASE_CONTROL_SHA" \
  --arg source_sha "$RELEASE_SOURCE_SHA" \
  --arg tag "$RELEASE_TAG_NAME" \
  --arg tag_object_sha "$RELEASE_TAG_OBJECT_SHA" \
  --arg tag_ref "$RELEASE_TAG_REF" \
  --arg version "$RELEASE_VERSION" \
  'type == "object" and (keys | sort) == ["approval", "contract_id", "control_sha", "create_draft", "final_draft", "from", "from_commit", "from_tag_object_sha", "prerelease", "range-kind", "release_notes_sha256", "repository", "request_run_id", "schema_version", "source_sha", "tag", "tag_object_sha", "tag_ref", "target", "title", "to", "version"] and .schema_version == 1 and .contract_id == "git.release-flow" and .approval == true and (.create_draft | type == "boolean") and (.final_draft | type == "boolean") and (.prerelease | type == "boolean") and .repository == $repository and .control_sha == $control_sha and .source_sha == $source_sha and .tag == $tag and .tag_ref == $tag_ref and .tag_object_sha == $tag_object_sha and .version == $version and .to == .tag and .target == "GitHub Release" and ([.request_run_id, .release_notes_sha256, .title] | all(type == "string" and length > 0)) and ((.["range-kind"] == "initial" and .from == null and .from_commit == null and .from_tag_object_sha == null) or (.["range-kind"] == "continuing" and ([.from, .from_commit, .from_tag_object_sha] | all(type == "string" and length > 0))))' \
  "$authority_file" >/dev/null || { echo "release authority is invalid" >&2; exit 1; }
release_notes_sha256="$(sha256sum "$release_notes_file" | awk '{print $1}')"
[[ "$release_notes_sha256" == "$(jq -r '.release_notes_sha256' "$authority_file")" ]] || { echo "release notes digest mismatch" >&2; exit 1; }
[[ -s "$release_notes_file" ]] || { echo "approved release notes are empty" >&2; exit 1; }
computed_authority_fingerprint="$( { jq -cS . "$authority_file"; cat "$release_notes_file"; } | sha256sum | awk '{print $1}')"
[[ "$computed_authority_fingerprint" == "$RELEASE_AUTHORITY_FINGERPRINT" ]] || { echo "release authority fingerprint mismatch" >&2; exit 1; }
release_range_kind="$(jq -r '.["range-kind"]' "$authority_file")"
release_from="$(jq -r '.from // empty' "$authority_file")"
release_from_commit="$(jq -r '.from_commit // empty' "$authority_file")"
release_from_tag_object_sha="$(jq -r '.from_tag_object_sha // empty' "$authority_file")"
release_title="$(jq -r '.title' "$authority_file")"
create_draft="$(jq -r '.create_draft' "$authority_file")"
final_draft="$(jq -r '.final_draft' "$authority_file")"
release_prerelease="$(jq -r '.prerelease' "$authority_file")"
contract_create_draft="$(jq -r '.publication_state.create_draft' "$release_assets_contract")"
contract_final_draft="$(jq -r '.publication_state.final_draft' "$release_assets_contract")"
contract_prerelease="$(jq -r '.publication_state.prerelease' "$release_assets_contract")"
[[ "$create_draft" == "$contract_create_draft" && "$final_draft" == "$contract_final_draft" && "$release_prerelease" == "$contract_prerelease" ]] || { echo "release authority state does not match the publication contract" >&2; exit 1; }

verify_continuing_release_state() {
  local latest_release_json from_ref
  [[ "$release_range_kind" == continuing ]] || return 0
  [[ "$release_from_commit" =~ ^[0-9a-f]{40}$ && "$release_from_tag_object_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid previous release tag identity" >&2; exit 1; }
  from_ref="refs/tags/$release_from"
  [[ "$(git -C "$control_root" rev-parse --verify "${from_ref}^{tag}")" == "$release_from_tag_object_sha" ]] || { echo "previous release tag object changed before publication" >&2; exit 1; }
  [[ "$(git -C "$control_root" rev-parse --verify "${from_ref}^{commit}")" == "$release_from_commit" ]] || { echo "previous release tag commit changed before publication" >&2; exit 1; }
  bash "$script_root/verify-remote-annotated-tag.sh" "$RELEASE_REPOSITORY" "$release_from" "$release_from_tag_object_sha" "$release_from_commit"
  latest_release_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/latest")" || { echo "previous stable GitHub Release state is unavailable" >&2; exit 1; }
  jq -e --arg from "$release_from" '.draft == false and .prerelease == false and .tag_name == $from' <<< "$latest_release_json" >/dev/null || { echo "release range start is no longer the latest stable GitHub Release" >&2; exit 1; }
}

staging_root="$RUNNER_TEMP/release-publication-$GITHUB_RUN_ID"
mkdir -p "$staging_root"
checksums_file="$staging_root/SHA256SUMS"
: > "$checksums_file"
expected_assets=()

verify_handoff() {
  local handoff_root="$1" expected_target="$2" manifest="$1/manifest.json" asset_name expected_sha actual_sha
  [[ -d "$handoff_root" && ! -L "$handoff_root" ]] || { echo "invalid handoff directory" >&2; exit 1; }
  [[ "$(find "$handoff_root" -mindepth 1 -maxdepth 1 | wc -l | tr -d ' ')" == 2 ]] || { echo "handoff must contain exactly two entries" >&2; exit 1; }
  [[ -f "$manifest" && ! -L "$manifest" ]] || { echo "missing handoff manifest" >&2; exit 1; }
  jq -e --arg source_sha "$RELEASE_SOURCE_SHA" --arg tag_ref "$RELEASE_TAG_REF" --arg target "$expected_target" --arg version "$RELEASE_VERSION" 'type == "object" and (keys | sort) == ["asset_name", "asset_sha256", "product", "schema_version", "source_sha", "tag_ref", "target", "version"] and .schema_version == 1 and .product == "a3-rust-cruiser" and .version == $version and .tag_ref == $tag_ref and .source_sha == $source_sha and .target == $target and (.asset_name | type == "string") and (.asset_sha256 | test("^[0-9a-f]{64}$"))' "$manifest" >/dev/null || { echo "invalid handoff manifest for $expected_target" >&2; exit 1; }
  asset_name="$(jq -r '.asset_name' "$manifest")"
  [[ "$asset_name" == "a3-rust-cruiser-v${RELEASE_VERSION}-${expected_target}.tar.gz" ]] || { echo "unexpected asset name" >&2; exit 1; }
  [[ -f "$handoff_root/$asset_name" && ! -L "$handoff_root/$asset_name" ]] || { echo "missing release asset" >&2; exit 1; }
  expected_sha="$(jq -r '.asset_sha256' "$manifest")"
  actual_sha="$(sha256sum "$handoff_root/$asset_name" | awk '{print $1}')"
  [[ "$actual_sha" == "$expected_sha" ]] || { echo "release asset checksum mismatch" >&2; exit 1; }
  printf '%s  %s\n' "$actual_sha" "$asset_name" >> "$checksums_file"
  cp "$handoff_root/$asset_name" "$staging_root/$asset_name"
  expected_assets+=("$asset_name")
}

jq -e --argjson expected "$(jq -c '[.platforms[].target]' "$release_assets_contract")" '. == $expected and length == 2' <<< "$EXPECTED_PLATFORMS" >/dev/null || { echo "unexpected release platform policy" >&2; exit 1; }
linux_target="$(jq -er '.platforms[] | select(.runner == "ubuntu-24.04") | .target' "$release_assets_contract")"
macos_target="$(jq -er '.platforms[] | select(.runner == "macos-15") | .target' "$release_assets_contract")"
verify_handoff "$linux_handoff" "$linux_target"
verify_handoff "$macos_handoff" "$macos_target"
sort -o "$checksums_file" "$checksums_file"
expected_assets+=(SHA256SUMS)

verify_asset_set() {
  local observed_release="$1" asset_name local_sha
  [[ "$(jq '.assets | length' <<< "$observed_release")" == 3 ]] || { echo "published asset count mismatch" >&2; exit 1; }
  for asset_name in "${expected_assets[@]}"; do
    local_sha="$(sha256sum "$staging_root/$asset_name" | awk '{print $1}')"
    jq -e --arg name "$asset_name" --arg digest "sha256:$local_sha" '[.assets[] | select(.name == $name and .digest == $digest)] | length == 1' <<< "$observed_release" >/dev/null || { echo "published asset digest mismatch: $asset_name" >&2; exit 1; }
  done
}

if [[ "$release_range_kind" == initial ]]; then
  bash "$script_root/verify-initial-release-state.sh" "$RELEASE_REPOSITORY" "$RELEASE_TAG_NAME"
fi
verify_continuing_release_state
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
release_count="$(jq --arg tag "$RELEASE_TAG_NAME" '[.[][] | select(.tag_name == $tag)] | length' <<< "$releases_json")"
[[ "$release_count" == 0 ]] || { echo "a release already exists for $RELEASE_TAG_NAME; refusing to overwrite it" >&2; exit 1; }
[[ "$(git -C "$control_root" rev-parse --verify "${RELEASE_TAG_REF}^{commit}")" == "$RELEASE_SOURCE_SHA" ]] || { echo "release tag changed during validation" >&2; exit 1; }
verify_live_tag

release_json="$(gh api --method POST -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases" -f tag_name="$RELEASE_TAG_NAME" -f target_commitish="$RELEASE_SOURCE_SHA" -f name="$release_title" -F "body=@$release_notes_file" -F draft="$create_draft" -F prerelease="$release_prerelease" -F generate_release_notes=false)"
printf '%s' "$release_json" | bash "$script_root/verify-release-body.sh" "$release_notes_file"
release_id="$(jq -r '.id' <<< "$release_json")"
[[ "$release_id" =~ ^[0-9]+$ ]] || { echo "GitHub did not return a release id" >&2; exit 1; }
jq -e --arg tag "$RELEASE_TAG_NAME" --arg title "$release_title" --argjson draft "$create_draft" --argjson prerelease "$release_prerelease" --argjson id "$release_id" '.id == $id and .tag_name == $tag and .name == $title and .draft == $draft and .prerelease == $prerelease' <<< "$release_json" >/dev/null || { echo "created release is not the expected draft" >&2; exit 1; }
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson draft "$create_draft" --argjson prerelease "$release_prerelease" --argjson id "$release_id" '. as $pages | ([$pages[][] | select(.tag_name == $tag and .id == $id and .draft == $draft and .prerelease == $prerelease)] | length == 1) and ([$pages[][] | select(.tag_name == $tag)] | length == 1)' <<< "$releases_json" >/dev/null || { echo "release identity is not unique after creation" >&2; exit 1; }
for asset_name in "${expected_assets[@]}"; do
  content_type=application/gzip; [[ "$asset_name" == SHA256SUMS ]] && content_type=text/plain
  gh api --method POST -H 'X-GitHub-Api-Version: 2022-11-28' -H "Content-Type: $content_type" --input "$staging_root/$asset_name" "https://uploads.github.com/repos/$RELEASE_REPOSITORY/releases/$release_id/assets?name=$asset_name" >/dev/null
done

release_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/$release_id")"
printf '%s' "$release_json" | bash "$script_root/verify-release-body.sh" "$release_notes_file"
jq -e --arg tag "$RELEASE_TAG_NAME" --arg title "$release_title" --argjson draft "$create_draft" --argjson prerelease "$release_prerelease" --argjson id "$release_id" '.id == $id and .tag_name == $tag and .name == $title and .draft == $draft and .prerelease == $prerelease' <<< "$release_json" >/dev/null || { echo "release changed before publication" >&2; exit 1; }
verify_asset_set "$release_json"
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson draft "$create_draft" --argjson id "$release_id" '. as $pages | ([$pages[][] | select(.tag_name == $tag and .id == $id and .draft == $draft)] | length == 1) and ([$pages[][] | select(.tag_name == $tag)] | length == 1)' <<< "$releases_json" >/dev/null || { echo "release is not unique before publication" >&2; exit 1; }
verify_live_tag
if [[ "$release_range_kind" == initial ]]; then
  bash "$script_root/verify-initial-release-state.sh" "$RELEASE_REPOSITORY" "$RELEASE_TAG_NAME"
fi
verify_continuing_release_state
if [[ "$create_draft" == true && "$final_draft" == false ]]; then
  release_json="$(gh api --method PATCH -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/$release_id" -F draft="$final_draft")"
fi
printf '%s' "$release_json" | bash "$script_root/verify-release-body.sh" "$release_notes_file"
jq -e --arg tag "$RELEASE_TAG_NAME" --arg title "$release_title" --argjson draft "$final_draft" --argjson prerelease "$release_prerelease" '.draft == $draft and .prerelease == $prerelease and .tag_name == $tag and .name == $title' <<< "$release_json" >/dev/null || { echo "release publication state mismatch" >&2; exit 1; }
verify_live_tag
release_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/$release_id")"
printf '%s' "$release_json" | bash "$script_root/verify-release-body.sh" "$release_notes_file"
jq -e --arg tag "$RELEASE_TAG_NAME" --arg title "$release_title" --argjson draft "$final_draft" --argjson prerelease "$release_prerelease" --argjson id "$release_id" '.id == $id and .draft == $draft and .prerelease == $prerelease and .tag_name == $tag and .name == $title' <<< "$release_json" >/dev/null || { echo "published release state mismatch" >&2; exit 1; }
verify_asset_set "$release_json"
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson draft "$final_draft" --argjson prerelease "$release_prerelease" --argjson id "$release_id" '. as $pages | ([$pages[][] | select(.tag_name == $tag and .id == $id and .draft == $draft and .prerelease == $prerelease)] | length == 1) and ([$pages[][] | select(.tag_name == $tag)] | length == 1)' <<< "$releases_json" >/dev/null || { echo "published release identity is not unique" >&2; exit 1; }
