#!/usr/bin/env bash

set -euo pipefail

control_root="${1:?trusted control checkout is required}"
linux_handoff="${2:?Linux handoff directory is required}"
macos_handoff="${3:?macOS handoff directory is required}"
required=(EXPECTED_LINUX_ARTIFACT_DIGEST EXPECTED_LINUX_ARTIFACT_ID EXPECTED_MACOS_ARTIFACT_DIGEST EXPECTED_MACOS_ARTIFACT_ID EXPECTED_PLATFORMS GITHUB_RUN_ID GH_TOKEN RELEASE_REPOSITORY RELEASE_SOURCE_SHA RELEASE_TAG_NAME RELEASE_TAG_REF RELEASE_VERSION RUNNER_TEMP SOURCE_POLICY_FINGERPRINT)
for name in "${required[@]}"; do [[ -n "${!name:-}" ]] || { echo "$name is required" >&2; exit 1; }; done

[[ "$RELEASE_TAG_REF" == "refs/tags/$RELEASE_TAG_NAME" && "$RELEASE_TAG_NAME" == "v$RELEASE_VERSION" ]] || { echo "release identity mismatch" >&2; exit 1; }
[[ "$EXPECTED_LINUX_ARTIFACT_ID" =~ ^[0-9]+$ && "$EXPECTED_MACOS_ARTIFACT_ID" =~ ^[0-9]+$ && "$GITHUB_RUN_ID" =~ ^[0-9]+$ ]] || { echo "artifact and run ids must be numeric" >&2; exit 1; }
[[ "$SOURCE_POLICY_FINGERPRINT" =~ ^[0-9a-f]{64}$ ]] || { echo "invalid policy fingerprint" >&2; exit 1; }
computed_policy_fingerprint="$(printf '%s\n%s\n%s\n' "$RELEASE_TAG_REF" "$RELEASE_SOURCE_SHA" "$EXPECTED_PLATFORMS" | sha256sum | awk '{print $1}')"
[[ "$computed_policy_fingerprint" == "$SOURCE_POLICY_FINGERPRINT" ]] || { echo "release policy fingerprint mismatch" >&2; exit 1; }
[[ "$(git -C "$control_root" rev-parse --verify "${RELEASE_TAG_REF}^{commit}")" == "$RELEASE_SOURCE_SHA" ]] || { echo "release tag changed before publication" >&2; exit 1; }

resolve_live_tag_commit() {
  local object_json object_sha object_type depth=0
  object_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/git/ref/tags/$RELEASE_TAG_NAME")"
  object_sha="$(jq -r '.object.sha' <<< "$object_json")"
  object_type="$(jq -r '.object.type' <<< "$object_json")"
  while [[ "$object_type" == tag ]]; do
    ((depth += 1))
    [[ "$depth" -le 10 ]] || { echo "annotated tag chain is too deep" >&2; exit 1; }
    object_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/git/tags/$object_sha")"
    object_sha="$(jq -r '.object.sha' <<< "$object_json")"
    object_type="$(jq -r '.object.type' <<< "$object_json")"
  done
  [[ "$object_type" == commit && "$object_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "release tag does not resolve to a commit" >&2; exit 1; }
  printf '%s\n' "$object_sha"
}

verify_live_tag() {
  [[ "$(resolve_live_tag_commit)" == "$RELEASE_SOURCE_SHA" ]] || { echo "live release tag does not match the verified source SHA" >&2; exit 1; }
}

verify_artifact() {
  local artifact_id="$1" expected_digest="$2" expected_name="$3" metadata
  metadata="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/actions/artifacts/$artifact_id")"
  jq -e --arg digest "$expected_digest" --arg name "$expected_name" --argjson run_id "$GITHUB_RUN_ID" '.expired == false and .name == $name and .digest == $digest and .workflow_run.id == $run_id' <<< "$metadata" >/dev/null || { echo "artifact identity or digest mismatch: $artifact_id" >&2; exit 1; }
}
verify_artifact "$EXPECTED_LINUX_ARTIFACT_ID" "$EXPECTED_LINUX_ARTIFACT_DIGEST" release-linux-x86_64
verify_artifact "$EXPECTED_MACOS_ARTIFACT_ID" "$EXPECTED_MACOS_ARTIFACT_DIGEST" release-macos-arm64

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

jq -e '. == ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]' <<< "$EXPECTED_PLATFORMS" >/dev/null || { echo "unexpected release platform policy" >&2; exit 1; }
verify_handoff "$linux_handoff" x86_64-unknown-linux-gnu
verify_handoff "$macos_handoff" aarch64-apple-darwin
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

releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
release_count="$(jq --arg tag "$RELEASE_TAG_NAME" '[.[][] | select(.tag_name == $tag)] | length' <<< "$releases_json")"
[[ "$release_count" == 0 ]] || { echo "a release already exists for $RELEASE_TAG_NAME; refusing to overwrite it" >&2; exit 1; }
[[ "$(git -C "$control_root" rev-parse --verify "${RELEASE_TAG_REF}^{commit}")" == "$RELEASE_SOURCE_SHA" ]] || { echo "release tag changed during validation" >&2; exit 1; }
verify_live_tag

release_json="$(gh api --method POST -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases" -f tag_name="$RELEASE_TAG_NAME" -f target_commitish="$RELEASE_SOURCE_SHA" -f name="$RELEASE_TAG_NAME" -f body="GitHub Release for a3-rust-cruiser $RELEASE_VERSION." -F draft=true -F prerelease=false -F generate_release_notes=false)"
release_id="$(jq -r '.id' <<< "$release_json")"
[[ "$release_id" =~ ^[0-9]+$ ]] || { echo "GitHub did not return a release id" >&2; exit 1; }
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson id "$release_id" '.id == $id and .tag_name == $tag and .draft == true and .prerelease == false' <<< "$release_json" >/dev/null || { echo "created release is not the expected draft" >&2; exit 1; }
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson id "$release_id" '. as $pages | ([$pages[][] | select(.tag_name == $tag and .id == $id and .draft == true and .prerelease == false)] | length == 1) and ([$pages[][] | select(.tag_name == $tag)] | length == 1)' <<< "$releases_json" >/dev/null || { echo "release identity is not a unique draft after creation" >&2; exit 1; }
for asset_name in "${expected_assets[@]}"; do
  content_type=application/gzip; [[ "$asset_name" == SHA256SUMS ]] && content_type=text/plain
  gh api --method POST -H 'X-GitHub-Api-Version: 2022-11-28' -H "Content-Type: $content_type" --input "$staging_root/$asset_name" "https://uploads.github.com/repos/$RELEASE_REPOSITORY/releases/$release_id/assets?name=$asset_name" >/dev/null
done

release_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/$release_id")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson id "$release_id" '.id == $id and .tag_name == $tag and .draft == true and .prerelease == false' <<< "$release_json" >/dev/null || { echo "release changed before publication" >&2; exit 1; }
verify_asset_set "$release_json"
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson id "$release_id" '. as $pages | ([$pages[][] | select(.tag_name == $tag and .id == $id and .draft == true)] | length == 1) and ([$pages[][] | select(.tag_name == $tag)] | length == 1)' <<< "$releases_json" >/dev/null || { echo "release is not a unique draft before publication" >&2; exit 1; }
verify_live_tag
release_json="$(gh api --method PATCH -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/$release_id" -F draft=false)"
jq -e --arg tag "$RELEASE_TAG_NAME" '.draft == false and .prerelease == false and .tag_name == $tag' <<< "$release_json" >/dev/null || { echo "release publication state mismatch" >&2; exit 1; }
verify_live_tag
release_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases/$release_id")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson id "$release_id" '.id == $id and .draft == false and .prerelease == false and .tag_name == $tag' <<< "$release_json" >/dev/null || { echo "published release state mismatch" >&2; exit 1; }
verify_asset_set "$release_json"
releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$RELEASE_REPOSITORY/releases?per_page=100")"
jq -e --arg tag "$RELEASE_TAG_NAME" --argjson id "$release_id" '. as $pages | ([$pages[][] | select(.tag_name == $tag and .id == $id and .draft == false and .prerelease == false)] | length == 1) and ([$pages[][] | select(.tag_name == $tag)] | length == 1)' <<< "$releases_json" >/dev/null || { echo "published release identity is not unique" >&2; exit 1; }
