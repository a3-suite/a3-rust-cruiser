#!/usr/bin/env bash

set -euo pipefail

repository="${1:?repository is required}"
tag_name="${2:?release tag is required}"

: "${GH_TOKEN:?GH_TOKEN is required}"

[[ "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "repository identity is invalid" >&2; exit 1; }
[[ "$tag_name" != *$'\n'* && "$tag_name" != *$'\r'* ]] || { echo "release tag identity is invalid" >&2; exit 1; }

tags_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$repository/tags?per_page=100")" || { echo "GitHub tag state is unavailable" >&2; exit 1; }
jq -e 'type == "array" and all(.[]; type == "array") and all(.[][]; (.name | type) == "string")' <<< "$tags_json" >/dev/null || { echo "GitHub tag state is invalid" >&2; exit 1; }
jq -e --arg target "$tag_name" '[.[][] | select(.name != $target)] | length == 0' <<< "$tags_json" >/dev/null || { echo "initial release requires no previous release tags" >&2; exit 1; }

releases_json="$(gh api --paginate --slurp -H 'X-GitHub-Api-Version: 2022-11-28' "repos/$repository/releases?per_page=100")" || { echo "GitHub release state is unavailable" >&2; exit 1; }
jq -e 'type == "array" and all(.[]; type == "array") and all(.[][]; (.draft | type) == "boolean" and (.prerelease | type) == "boolean" and (.tag_name | type) == "string")' <<< "$releases_json" >/dev/null || { echo "GitHub release state is invalid" >&2; exit 1; }
jq -e '[.[][] | select(.draft == false and .prerelease == false)] | length == 0' <<< "$releases_json" >/dev/null || { echo "initial release requires no published stable GitHub Releases" >&2; exit 1; }
