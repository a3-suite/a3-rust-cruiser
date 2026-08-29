#!/usr/bin/env bash

set -euo pipefail

repository="${1:?repository is required}"
tag_name="${2:?tag name is required}"
expected_tag_object_sha="${3:?expected tag object SHA is required}"
expected_commit_sha="${4:?expected commit SHA is required}"

: "${GH_TOKEN:?GH_TOKEN is required}"

[[ "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "repository identity is invalid" >&2; exit 1; }
[[ "$tag_name" != *$'\n'* && "$tag_name" != *$'\r'* ]] || { echo "tag identity is invalid" >&2; exit 1; }
[[ "$expected_tag_object_sha" =~ ^[0-9a-f]{40}$ && "$expected_commit_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "expected tag identity must use full SHAs" >&2; exit 1; }

ref_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$repository/git/ref/tags/$tag_name")" || { echo "remote tag ref is unavailable" >&2; exit 1; }
jq -e --arg sha "$expected_tag_object_sha" '.object.type == "tag" and .object.sha == $sha' <<< "$ref_json" >/dev/null || { echo "remote tag object identity mismatch" >&2; exit 1; }

tag_json="$(gh api -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$repository/git/tags/$expected_tag_object_sha")" || { echo "remote tag object is unavailable" >&2; exit 1; }
jq -e \
  --arg commit "$expected_commit_sha" \
  --arg tag "$tag_name" \
  '.tag == $tag and .message == $tag and .object.type == "commit" and .object.sha == $commit' \
  <<< "$tag_json" >/dev/null || { echo "remote annotated tag identity mismatch" >&2; exit 1; }
