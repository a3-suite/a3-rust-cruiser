#!/usr/bin/env bash

set -euo pipefail

script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

bash "$script_root/ci-workflow-contract-test.sh"
bash "$script_root/installer-standard-contract-test.sh"
