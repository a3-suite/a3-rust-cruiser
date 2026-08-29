#!/usr/bin/env bash

set -euo pipefail

approved_notes_file="${1:?approved release notes file is required}"

[[ -f "$approved_notes_file" && ! -L "$approved_notes_file" ]] || { echo "approved release notes must be a regular file" >&2; exit 1; }
release_json="$(cat)"
jq -e '.body | type == "string"' <<< "$release_json" >/dev/null || { echo "release body is missing" >&2; exit 1; }
approved_sha256="$(sha256sum "$approved_notes_file" | awk '{print $1}')"
observed_sha256="$(jq -j '.body' <<< "$release_json" | sha256sum | awk '{print $1}')"
[[ "$observed_sha256" == "$approved_sha256" ]] || { echo "published release notes differ from approved notes" >&2; exit 1; }
