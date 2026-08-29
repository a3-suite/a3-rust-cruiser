#!/usr/bin/env bash

set -euo pipefail

# evidence_role: contract
# contract_id: ci.release-asset-publication-contract
# integration_id: release-request-workflow-binding

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
request_workflow="$repository_root/.github/workflows/validate.yml"
publish_workflow="$repository_root/.github/workflows/publish-release.yml"
publication_contract="$repository_root/.ci/release-assets.json"
release_validator="$repository_root/.ci/scripts/validate-release-tag.sh"

# GitHub expressions are literal strings in these contract assertions.
# shellcheck disable=SC2016
grep -F 'REQUEST_REF: ${{ github.ref }}' "$request_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'REQUEST_REF_TYPE: ${{ github.ref_type }}' "$request_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'REQUEST_TITLE: ${{ inputs.title }}' "$request_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'REQUEST_CREATE_DRAFT: ${{ toJSON(inputs.create_draft) }}' "$request_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'REQUEST_FINAL_DRAFT: ${{ toJSON(inputs.final_draft) }}' "$request_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'REQUEST_PRERELEASE: ${{ toJSON(inputs.prerelease) }}' "$request_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'ref: ${{ github.sha }}' "$publish_workflow" >/dev/null
# shellcheck disable=SC2016
grep -F 'group: publish-release-${{ github.repository }}' "$publish_workflow" >/dev/null
grep -F 'cancel-in-progress: false' "$publish_workflow" >/dev/null
grep -F 'queue: max' "$publish_workflow" >/dev/null
[[ -f "$publication_contract" && ! -L "$publication_contract" ]]

jq -e '
  type == "object" and
  (keys | sort) == ["platforms", "publication_state", "publication_target", "schema_version"] and
  .schema_version == 1 and
  .publication_target == "GitHub Release" and
  .publication_state == {create_draft: true, final_draft: false, prerelease: false} and
  [.platforms[].target] == ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"] and
  [.platforms[].runner] == ["ubuntu-24.04", "macos-15"]
' "$publication_contract" >/dev/null

# Project publication values are owned by release-assets.json. The validator
# checks the contract shape without restating those values.
if grep -F '.publication_state == {' "$release_validator" >/dev/null; then
  echo "release validator must not redefine publication state values" >&2
  exit 1
fi
if grep -F '.platforms == [' "$release_validator" >/dev/null; then
  echo "release validator must not redefine publication platform values" >&2
  exit 1
fi
grep -F '([.[].target] | unique | length) == 2' "$release_validator" >/dev/null

linux_runner="$(jq -r '.platforms[] | select(.target == "x86_64-unknown-linux-gnu") | .runner' "$publication_contract")"
macos_runner="$(jq -r '.platforms[] | select(.target == "aarch64-apple-darwin") | .runner' "$publication_contract")"
linux_job="$(sed -n '/^  build-linux:/,/^  build-macos:/p' "$publish_workflow")"
macos_job="$(sed -n '/^  build-macos:/,/^  release:/p' "$publish_workflow")"
grep -F "runs-on: $linux_runner" <<< "$linux_job" >/dev/null
grep -F "x86_64-unknown-linux-gnu" <<< "$linux_job" >/dev/null
grep -F "runs-on: $macos_runner" <<< "$macos_job" >/dev/null
grep -F "aarch64-apple-darwin" <<< "$macos_job" >/dev/null

echo "release workflow contract tests passed"
