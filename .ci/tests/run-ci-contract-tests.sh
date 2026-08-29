#!/usr/bin/env bash

set -euo pipefail

script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

bash "$script_root/runner-trust-contract-test.sh"
bash "$script_root/ci-tool-version-contract-test.sh"
bash "$script_root/release-authority-contract-test.sh"
bash "$script_root/release-workflow-contract-test.sh"
