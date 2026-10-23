#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
workflow_root="$repository_root/.github/workflows"
ci_root="$repository_root/.ci"

# consumer 資産は薄い caller 4 本。provider の reusable workflow は
# consumer 配置資産ではないため release-publication.yml を要求しない。
for workflow in quality-gate release-request-tag release-publication-request release-publication-caller; do
  test -f "$workflow_root/$workflow.yml"
done

for workflow in release-publication ci-post-merge ci-trusted ci-untrusted-pr validate publish-release; do
  test ! -e "$workflow_root/$workflow.yml"
done

for retired in \
  "$ci_root/trusted/ci-release-authority.sh" \
  "$ci_root/trusted/ci-release-assemble.sh" \
  "$ci_root/trusted/ci-release-publish.sh" \
  "$ci_root/scripts/ci-source-gate.sh" \
  "$ci_root/scripts/ci-release-build.sh" \
  "$ci_root/adapters/rust-quality.yml"; do
  test ! -e "$retired"
done

if grep -R -nE '<[a-z][a-z0-9-]*>' "$workflow_root" >/dev/null; then
  echo "canonical workflow placeholder remains" >&2
  exit 1
fi

# 外部 Action と provider reusable workflow は完全長 commit SHA に固定する。
if grep -R -nE 'uses:[[:space:]]+[^ ]+@(v[0-9]|main|master|HEAD)' "$workflow_root" >/dev/null; then
  echo "mutable Action reference remains" >&2
  exit 1
fi
while IFS= read -r reference; do
  [[ "$reference" =~ @[a-f0-9]{40}$ ]] || {
    echo "unpinned Action reference: $reference" >&2
    exit 1
  }
done < <(grep -RhEo 'uses:[[:space:]]+[^ ]+@[^ ]+' "$workflow_root" | sed -E 's/^uses:[[:space:]]+//')

# 配布同一性: lock の source revision と caller の callee 参照が一致する。
lock_source=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["sourceRevision"])' "$ci_root/ci-assets.lock.json")
[[ "$lock_source" =~ ^[a-f0-9]{40}$ ]]

# lock の資産集合と適用 byte が作業 tree と一致する。
python3 - "$ci_root/ci-assets.lock.json" "$repository_root" <<'PY'
import hashlib, json, sys
from pathlib import Path

lock = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
root = Path(sys.argv[2])
expected = {
    ".ci/provider-action-pins.yml",
    ".github/workflows/quality-gate.yml",
    ".github/workflows/release-publication-caller.yml",
    ".github/workflows/release-publication-request.yml",
    ".github/workflows/release-request-tag.yml",
}
paths = {asset["path"] for asset in lock["assets"]}
if paths != expected:
    raise SystemExit(f"unexpected lock asset set: {sorted(paths)}")
for asset in lock["assets"]:
    digest = hashlib.sha256((root / asset["path"]).read_bytes()).hexdigest()
    if digest != asset["appliedSha256"]:
        raise SystemExit(f"applied digest mismatch: {asset['path']}")
PY

# Rust toolchain は rust-toolchain.toml を正本とし、caller 初期パラメータとの乖離を防ぐ。
toolchain_channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repository_root/rust-toolchain.toml")
test -n "$toolchain_channel"
for workflow in quality-gate release-publication-caller; do
  test "$(sed -n 's/^      toolchain-version: \(.*\)$/\1/p' "$workflow_root/$workflow.yml" | head -1)" = "$toolchain_channel"
done

grep -Fx 'platforms:' "$ci_root/platform-manifest.yml" >/dev/null
test "$(grep -c '^  - id:' "$ci_root/platform-manifest.yml")" = 3
grep -Fx '    runner: ubuntu-24.04' "$ci_root/platform-manifest.yml" >/dev/null
grep -Fx '    runner: macos-15' "$ci_root/platform-manifest.yml" >/dev/null
grep -Fx '    runner: windows-2022' "$ci_root/platform-manifest.yml" >/dev/null
grep -Fx '    target: x86_64-unknown-linux-gnu' "$ci_root/platform-manifest.yml" >/dev/null
grep -Fx '    target: aarch64-apple-darwin' "$ci_root/platform-manifest.yml" >/dev/null
grep -Fx '    target: x86_64-pc-windows-msvc' "$ci_root/platform-manifest.yml" >/dev/null

quality_gate_contract() {
  grep -F -x '  pull_request: null' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F -x '    branches:' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F -x '      - main' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F "a3-suite/a3-ci-github/.github/workflows/ci-quality.yml@$lock_source" "$workflow_root/quality-gate.yml" >/dev/null
  grep -F 'language-profile: rust' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F 'standard-bundle-id: rust-cargo-quality' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F 'adapter-descriptor: ""' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F 'docs-only-patterns: docs/**,README.md,**/*.md' "$workflow_root/quality-gate.yml" >/dev/null
  grep -F 'ci-quality-summary@' "$workflow_root/quality-gate.yml" >/dev/null
}

release_request_contract() {
  grep -F -x '  push:' "$workflow_root/release-request-tag.yml" >/dev/null
  grep -F -x '    tags:' "$workflow_root/release-request-tag.yml" >/dev/null
  grep -F 'ci-annotated-tag-resolver@' "$workflow_root/release-request-tag.yml" >/dev/null
  grep -F 'ci-release-request-handoff@' "$workflow_root/release-request-tag.yml" >/dev/null
  grep -F '      - v*' "$workflow_root/release-request-tag.yml" >/dev/null
  if grep -nE 'contents:[[:space:]]*write' "$workflow_root/release-request-tag.yml" >/dev/null; then
    echo "tag request must not hold contents write" >&2
    exit 1
  fi
}

release_publication_contract() {
  grep -F -x '  workflow_run:' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F "a3-suite/a3-ci-github/.github/workflows/ci-release-publication.yml@$lock_source" "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'contents: write' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'workflow_run:' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'release-publication-request' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'language-profile: rust' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'platform-manifest: .ci/platform-manifest.yml' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'release-implementation: rust-cli-release' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'supplemental_release_asset_enabled: true' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'supplemental_release_asset_owner_contract: installer.asset-assembly-evidence-contract' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'supplemental_release_asset_implementation: standard-installer' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'supplemental_release_asset_config_path: .ci/installer/assembly.json' "$workflow_root/release-publication-caller.yml" >/dev/null
  grep -F 'release_identity:' "$workflow_root/release-publication-request.yml" >/dev/null
  grep -F 'approval_id:' "$workflow_root/release-publication-request.yml" >/dev/null
  grep -F 'approval_body_sha256:' "$workflow_root/release-publication-request.yml" >/dev/null
  if grep -nE 'contents:[[:space:]]*write' "$workflow_root/release-publication-request.yml" >/dev/null; then
    echo "publication request must not hold contents write" >&2
    exit 1
  fi
  test -f "$repository_root/.ci/installer/assembly.json"
  grep -F 'release_skills_config_path: .ci/release-skills.json' "$workflow_root/release-publication-caller.yml" >/dev/null
  # 公開スキルの宣言が実在する root と必須 file を指し、Release ZIP の収録漏れを公開前に検出する。
  python3 - "$ci_root/release-skills.json" "$repository_root" <<'PY'
import json, sys
from pathlib import Path

declaration = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
root = Path(sys.argv[2])
if declaration.get("schemaVersion") != "ci.release-skills.v1" or not declaration.get("skills"):
    raise SystemExit("invalid release skills declaration")
for skill in declaration["skills"]:
    skill_root = root / skill["path"]
    if not (skill_root / "SKILL.md").is_file():
        raise SystemExit(f"release skill root has no SKILL.md: {skill['path']}")
    for required in skill["requiredFiles"]:
        if not (skill_root / required).is_file():
            raise SystemExit(f"release skill required file is missing: {skill['path']}/{required}")
        if not any(required == item or required.startswith(f"{item}/") for item in skill["include"]):
            raise SystemExit(f"release skill required file is not included: {skill['path']}/{required}")
PY
}

quality_gate_contract
release_request_contract
release_publication_contract

echo "CI workflow contract tests passed"
