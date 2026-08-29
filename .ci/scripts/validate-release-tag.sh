#!/usr/bin/env bash

set -euo pipefail

request_file="${1:?release request JSON is required}"
control_root="${2:?trusted control checkout is required}"
authority_output="${3:?release authority output directory is required}"
script_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

: "${EXPECTED_CONTROL_SHA:?EXPECTED_CONTROL_SHA is required}"
: "${EXPECTED_EVENT:?EXPECTED_EVENT is required}"
: "${EXPECTED_REPOSITORY:?EXPECTED_REPOSITORY is required}"
: "${EXPECTED_RUN_ID:?EXPECTED_RUN_ID is required}"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"

[[ -f "$request_file" && ! -L "$request_file" ]] || { echo "release request must be a regular file" >&2; exit 1; }
jq -e '
  type == "object" and
  (keys | sort) == ["approval", "control_sha", "event", "from", "range-kind", "release_notes", "repository", "run_id", "tag", "target", "target_commit", "to", "version"] and
  .approval == true and
  ([.control_sha, .event, .release_notes, .repository, .run_id, .tag, .target, .target_commit, .to, .version] | all(type == "string" and length > 0)) and
  ((.["range-kind"] == "initial" and .from == null) or (.["range-kind"] == "continuing" and (.from | type == "string" and length > 0)))
' "$request_file" >/dev/null || { echo "release request has an invalid or unapproved schema" >&2; exit 1; }

request_event="$(jq -r '.event' "$request_file")"
control_sha="$(jq -r '.control_sha' "$request_file")"
release_range_kind="$(jq -r '.["range-kind"]' "$request_file")"
release_from="$(jq -r '.from // empty' "$request_file")"
request_repository="$(jq -r '.repository' "$request_file")"
request_run_id="$(jq -r '.run_id' "$request_file")"
tag_name="$(jq -r '.tag' "$request_file")"
release_target="$(jq -r '.target' "$request_file")"
source_sha="$(jq -r '.target_commit' "$request_file")"
release_to="$(jq -r '.to' "$request_file")"
release_version="$(jq -r '.version' "$request_file")"
tag_ref="refs/tags/$tag_name"

[[ "$request_event" == "$EXPECTED_EVENT" ]] || { echo "request event mismatch" >&2; exit 1; }
[[ "$request_repository" == "$EXPECTED_REPOSITORY" ]] || { echo "request repository mismatch" >&2; exit 1; }
[[ "$request_run_id" == "$EXPECTED_RUN_ID" ]] || { echo "request run id mismatch" >&2; exit 1; }
[[ "$control_sha" == "$EXPECTED_CONTROL_SHA" ]] || { echo "request control SHA mismatch" >&2; exit 1; }
[[ "$control_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "control SHA must be a full commit SHA" >&2; exit 1; }
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "source SHA must be a full commit SHA" >&2; exit 1; }
[[ "$tag_name" == "v$release_version" ]] || { echo "release tag must match v{version}" >&2; exit 1; }
[[ "$release_to" == "$tag_name" ]] || { echo "release range endpoint must match the approved tag" >&2; exit 1; }
[[ "$release_target" == "GitHub Release" ]] || { echo "release target does not match this publisher" >&2; exit 1; }
git -C "$control_root" check-ref-format "$tag_ref" >/dev/null || { echo "approved tag is not a valid Git ref" >&2; exit 1; }

verify_tag_annotation() {
  local tag_object_sha="$1" expected_name="$2" role="$3" observed_sha expected_sha
  observed_sha="$(git -C "$control_root" cat-file tag "$tag_object_sha" | sed '1,/^$/d' | sha256sum | awk '{print $1}')"
  expected_sha="$(printf '%s\n' "$expected_name" | sha256sum | awk '{print $1}')"
  [[ "$observed_sha" == "$expected_sha" ]] || { echo "$role tag annotation must exactly match the tag name" >&2; exit 1; }
}

if [[ "$release_range_kind" == initial ]]; then
  bash "$script_root/verify-initial-release-state.sh" "$request_repository" "$tag_name"
  release_range_start=initial
else
  from_ref="refs/tags/$release_from"
  git -C "$control_root" check-ref-format "$from_ref" >/dev/null || { echo "release range start is not a valid Git ref" >&2; exit 1; }
  from_tag_object_sha="$(git -C "$control_root" rev-parse --verify "${from_ref}^{tag}" 2>/dev/null)" || { echo "release range start must be an existing annotated tag" >&2; exit 1; }
  [[ "$from_tag_object_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "release range start tag object SHA must be a full SHA" >&2; exit 1; }
  verify_tag_annotation "$from_tag_object_sha" "$release_from" "release range start"
  from_commit="$(git -C "$control_root" rev-parse --verify "${from_ref}^{commit}")"
  [[ "$from_commit" != "$source_sha" ]] || { echo "release range start must be older than the target commit" >&2; exit 1; }
  git -C "$control_root" merge-base --is-ancestor "$from_commit" "$source_sha" || { echo "release range start must be an ancestor of the target commit" >&2; exit 1; }
  latest_release_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$request_repository/releases/latest")" || { echo "previous stable GitHub Release state is unavailable" >&2; exit 1; }
  jq -e --arg from "$release_from" '.draft == false and .prerelease == false and .tag_name == $from' <<< "$latest_release_json" >/dev/null || { echo "release range start must match the previous stable GitHub Release tag" >&2; exit 1; }
  bash "$script_root/verify-remote-annotated-tag.sh" "$request_repository" "$release_from" "$from_tag_object_sha" "$from_commit"
  release_range_start="$release_from"
fi

tag_object_type="$(git -C "$control_root" cat-file -t "$tag_ref")"
[[ "$tag_object_type" == tag ]] || { echo "release tag must be annotated" >&2; exit 1; }
tag_object_sha="$(git -C "$control_root" rev-parse --verify "${tag_ref}^{tag}")"
[[ "$tag_object_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "tag object SHA must be a full commit SHA" >&2; exit 1; }
verify_tag_annotation "$tag_object_sha" "$tag_name" "release"
resolved_tag="$(git -C "$control_root" rev-parse --verify "${tag_ref}^{commit}")"
[[ "$resolved_tag" == "$source_sha" ]] || { echo "tag does not resolve to the requested source SHA" >&2; exit 1; }

package_version="$(git -C "$control_root" show "${source_sha}:Cargo.toml" | awk '/^\[package\][[:space:]]*$/ { p=1; next } /^\[/ { p=0 } p && /^[[:space:]]*version[[:space:]]*=/ { v=$0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/"[[:space:]]*$/, "", v); print v; exit }')"
[[ "$package_version" == "$release_version" ]] || { echo "tag version does not match Cargo.toml" >&2; exit 1; }

expected_platforms='["x86_64-unknown-linux-gnu","aarch64-apple-darwin"]'
[[ ! -e "$authority_output" ]] || { echo "release authority output already exists" >&2; exit 1; }
mkdir -p "$authority_output"
release_notes_file="$authority_output/release-notes.md"
authority_file="$authority_output/release-authority.json"
jq -j '.release_notes' "$request_file" > "$release_notes_file"
expected_release_metadata="$(printf '## リリース\n - Version: %s\n - Tag: %s\n - Range: %s -> %s' "$release_version" "$tag_name" "$release_range_start" "$release_to")"
observed_release_metadata="$(sed -n '1,4p' "$release_notes_file")"
[[ "$observed_release_metadata" == "$expected_release_metadata" ]] || { echo "release notes metadata does not match the approved release inputs" >&2; exit 1; }
release_notes_sha256="$(sha256sum "$release_notes_file" | awk '{print $1}')"
jq -n \
  --arg control_sha "$control_sha" \
  --arg from "$release_from" \
  --arg from_commit "${from_commit:-}" \
  --arg from_tag_object_sha "${from_tag_object_sha:-}" \
  --arg range_kind "$release_range_kind" \
  --arg release_notes_sha256 "$release_notes_sha256" \
  --arg repository "$request_repository" \
  --arg request_run_id "$request_run_id" \
  --arg source_sha "$source_sha" \
  --arg tag "$tag_name" \
  --arg tag_object_sha "$tag_object_sha" \
  --arg tag_ref "$tag_ref" \
  --arg target "$release_target" \
  --arg to "$release_to" \
  --arg version "$release_version" \
  '{schema_version: 1, contract_id: "git.release-flow", approval: true, repository: $repository, request_run_id: $request_run_id, control_sha: $control_sha, version: $version, "range-kind": $range_kind, from: (if $range_kind == "initial" then null else $from end), from_commit: (if $range_kind == "initial" then null else $from_commit end), from_tag_object_sha: (if $range_kind == "initial" then null else $from_tag_object_sha end), to: $to, tag: $tag, tag_ref: $tag_ref, tag_object_sha: $tag_object_sha, source_sha: $source_sha, target: $target, release_notes_sha256: $release_notes_sha256}' \
  > "$authority_file"
authority_fingerprint="$( { jq -cS . "$authority_file"; cat "$release_notes_file"; } | sha256sum | awk '{print $1}')"
{
  printf 'authority_fingerprint=%s\n' "$authority_fingerprint"
  printf 'control_sha=%s\n' "$control_sha"
  printf 'expected_platforms=%s\n' "$expected_platforms"
  printf 'release_version=%s\n' "$release_version"
  printf 'source_sha=%s\n' "$source_sha"
  printf 'tag_name=%s\n' "$tag_name"
  printf 'tag_object_sha=%s\n' "$tag_object_sha"
  printf 'tag_ref=%s\n' "$tag_ref"
} >> "$GITHUB_OUTPUT"
