#!/usr/bin/env bash

set -euo pipefail

request_file="${1:?release request JSON is required}"
control_root="${2:?trusted control checkout is required}"

: "${EXPECTED_EVENT:?EXPECTED_EVENT is required}"
: "${EXPECTED_REPOSITORY:?EXPECTED_REPOSITORY is required}"
: "${EXPECTED_RUN_ID:?EXPECTED_RUN_ID is required}"
: "${EXPECTED_SOURCE_SHA:?EXPECTED_SOURCE_SHA is required}"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"

[[ -f "$request_file" && ! -L "$request_file" ]] || { echo "release request must be a regular file" >&2; exit 1; }
jq -e 'type == "object" and (keys | sort) == ["event", "ref", "repository", "run_id", "source_sha"] and ([.event, .ref, .repository, .run_id, .source_sha] | all(type == "string"))' "$request_file" >/dev/null || { echo "release request has an invalid schema" >&2; exit 1; }

request_event="$(jq -r '.event' "$request_file")"
tag_ref="$(jq -r '.ref' "$request_file")"
request_repository="$(jq -r '.repository' "$request_file")"
request_run_id="$(jq -r '.run_id' "$request_file")"
source_sha="$(jq -r '.source_sha' "$request_file")"

[[ "$request_event" == "$EXPECTED_EVENT" ]] || { echo "request event mismatch" >&2; exit 1; }
[[ "$request_repository" == "$EXPECTED_REPOSITORY" ]] || { echo "request repository mismatch" >&2; exit 1; }
[[ "$request_run_id" == "$EXPECTED_RUN_ID" ]] || { echo "request run id mismatch" >&2; exit 1; }
[[ "$source_sha" == "$EXPECTED_SOURCE_SHA" ]] || { echo "request source SHA mismatch" >&2; exit 1; }
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "source SHA must be a full commit SHA" >&2; exit 1; }
[[ "$tag_ref" =~ ^refs/tags/v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || { echo "tag must have the form refs/tags/vX.Y.Z" >&2; exit 1; }

tag_name="${tag_ref#refs/tags/}"
release_version="${tag_name#v}"
resolved_tag="$(git -C "$control_root" rev-parse --verify "${tag_ref}^{commit}")"
[[ "$resolved_tag" == "$source_sha" ]] || { echo "tag does not resolve to the requested source SHA" >&2; exit 1; }

reachable=false
while IFS= read -r allowed_ref; do
  if git -C "$control_root" merge-base --is-ancestor "$source_sha" "$allowed_ref"; then reachable=true; break; fi
done < <(git -C "$control_root" for-each-ref --format='%(refname)' refs/remotes/origin/main refs/remotes/origin/hotfix/)
[[ "$reachable" == true ]] || { echo "release commit is not reachable from main or hotfix" >&2; exit 1; }

package_version="$(git -C "$control_root" show "${source_sha}:Cargo.toml" | awk '/^\[package\][[:space:]]*$/ { p=1; next } /^\[/ { p=0 } p && /^[[:space:]]*version[[:space:]]*=/ { v=$0; sub(/^[^=]*=[[:space:]]*"/, "", v); sub(/"[[:space:]]*$/, "", v); print v; exit }')"
[[ "$package_version" == "$release_version" ]] || { echo "tag version does not match Cargo.toml" >&2; exit 1; }

expected_platforms='["x86_64-unknown-linux-gnu","aarch64-apple-darwin"]'
source_policy_fingerprint="$(printf '%s\n%s\n%s\n' "$tag_ref" "$source_sha" "$expected_platforms" | sha256sum | awk '{print $1}')"
{
  printf 'expected_platforms=%s\n' "$expected_platforms"
  printf 'release_version=%s\n' "$release_version"
  printf 'source_policy_fingerprint=%s\n' "$source_policy_fingerprint"
  printf 'source_sha=%s\n' "$source_sha"
  printf 'tag_name=%s\n' "$tag_name"
  printf 'tag_ref=%s\n' "$tag_ref"
} >> "$GITHUB_OUTPUT"
