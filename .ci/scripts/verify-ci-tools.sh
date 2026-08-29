#!/usr/bin/env bash

set -euo pipefail

runner_label="${1:?runner label is required}"

rustup_version="$(rustup --version | awk 'NR == 1 { print $2; exit }')"
[[ "$rustup_version" == 1.29.0 ]] || {
  echo "rustup version mismatch: expected 1.29.0, got ${rustup_version:-<none>}" >&2
  exit 1
}

jq_version="$(jq --version)"
case "$runner_label" in
  ubuntu-24.04)
    [[ "$jq_version" == jq-1.7 ]] || {
      echo "jq version mismatch: expected jq-1.7, got ${jq_version:-<none>}" >&2
      exit 1
    }
    gh_version="$(gh --version | awk 'NR == 1 { print $3; exit }')"
    [[ "$gh_version" == 2.96.0 ]] || {
      echo "GitHub CLI version mismatch: expected 2.96.0, got ${gh_version:-<none>}" >&2
      exit 1
    }
    ;;
  macos-15)
    [[ "$jq_version" == jq-1.8.2 ]] || {
      echo "jq version mismatch: expected jq-1.8.2, got ${jq_version:-<none>}" >&2
      exit 1
    }
    ;;
  *)
    echo "unsupported runner label: $runner_label" >&2
    exit 1
    ;;
esac
