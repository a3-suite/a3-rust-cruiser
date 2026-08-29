#!/usr/bin/env bash

set -euo pipefail

# evidence_role: contract
# contract_id: ci.runner-trust-policy
# integration_id: pull-request-check-selection

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
trusted_workflow="$repository_root/.github/workflows/ci-trusted.yml"
untrusted_workflow="$repository_root/.github/workflows/ci-untrusted-pr.yml"

[[ -f "$trusted_workflow" ]]
grep -F 'github.event.pull_request.head.repo.full_name == github.repository' "$trusted_workflow" >/dev/null
# The GitHub expression is a literal string in this contract assertion.
# shellcheck disable=SC2016
grep -F 'ref: ${{ github.event.pull_request.base.sha }}' "$trusted_workflow" >/dev/null
grep -F 'path: control' "$trusted_workflow" >/dev/null
grep -F '.github/workflows' "$trusted_workflow" >/dev/null
grep -F 'github.event.pull_request.head.repo.full_name != github.repository' "$untrusted_workflow" >/dev/null
if grep -F 'root=source' "$trusted_workflow" >/dev/null; then
  echo "trusted CI must fail closed when base controls are unavailable" >&2
  exit 1
fi
grep -F 'if [[ -f base/.github/workflows/ci-trusted.yml ]]; then' "$untrusted_workflow" >/dev/null
grep -F 'steps.bootstrap.outputs.required == '\''true'\''' "$untrusted_workflow" >/dev/null

if grep -F 'branches:' "$trusted_workflow" "$untrusted_workflow" >/dev/null; then
  echo "PR merge-gate workflows must not use branch filters" >&2
  exit 1
fi

echo "runner trust contract tests passed"
