#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"

installer_standard_contract() {
python3 - "$repository_root" <<'PY'
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
project_version = re.search(
    r'^version = "([^"]+)"$',
    (root / "Cargo.toml").read_text(encoding="utf-8"),
    re.MULTILINE,
)
assert project_version, "Cargo.toml package version is missing"
release_version = project_version.group(1)
verification = (root / "sdd/dsl/specs/contract-core/subjects/installer-product-setup/verifications.sdd.yml").read_text(encoding="utf-8")
case_release_version = re.search(
    r"(?m)^        release_version: (.+)$",
    verification,
)
assert case_release_version, "installer contract case release version is missing"
assert case_release_version.group(1) == release_version, "installer contract case version differs from Cargo.toml"
assembly = json.loads((root / "installer/assembly.json").read_text(encoding="utf-8"))
readme = (root / "README.md").read_text(encoding="utf-8")
setup_guide = (root / "skills/a3-rust-cruiser/references/setup-release.guide.md").read_text(encoding="utf-8")
operations_guide = (root / "docs/operations/setup-release.md").read_text(encoding="utf-8")
expected = {
    "linux-x86_64": ("installer/manifests/linux-x86_64.json", "install-linux-x86_64.sh"),
    "macos-arm64": ("installer/manifests/macos-arm64.json", "install-macos-arm64.sh"),
    "windows-x86_64": ("installer/manifests/windows-x86_64.json", "install.ps1"),
}
assert assembly["schemaVersion"] == "installer.assembly.v1"
assert assembly["useCase"] == "github-release-native-shared"
assert assembly["verificationProfile"] == "shared-exact-payload-v1"
assert set(assembly["platforms"]) == set(expected)
assert assembly["sharedWrapper"] == {
    "assetName": "install.sh",
    "deliveryUrls": {
        "linux-x86_64": f"https://github.com/a3-suite/a3-rust-cruiser/releases/download/v{release_version}/install-linux-x86_64.sh",
        "macos-arm64": f"https://github.com/a3-suite/a3-rust-cruiser/releases/download/v{release_version}/install-macos-arm64.sh",
    },
}
assert "docs/operations/setup-release.md" in readme
assert f"/download/v{release_version}/install.sh" in readme
assert f"/download/v{release_version}/install.ps1" in readme
assert "Windows PowerShell 5.1 以降" in readme
assert 'powershell -ExecutionPolicy Bypass -c "irm ' in readme
assert "~/.local/bin/a3-rust-cruiser" in readme
assert 'export PATH="$HOME/.local/bin:$PATH"' in readme
assert '$env:Path = "$env:LOCALAPPDATA\\Programs\\a3-suite\\a3-rust-cruiser\\bin;$env:Path"' in readme
assert "a3-rust-cruiser --version" in readme
assert "~/.local/share/a3-suite/a3-rust-cruiser/standalone" in readme
assert "~/Library/Application Support/a3-suite/a3-rust-cruiser/standalone" in readme
assert "%LOCALAPPDATA%\\a3-suite\\a3-rust-cruiser\\standalone" in readme
assert "%LOCALAPPDATA%\\Programs\\a3-suite\\a3-rust-cruiser\\bin\\a3-rust-cruiser.exe" in readme
assert "--source offline" in operations_guide
assert "-Source offline" in operations_guide
assert "/download/" not in operations_guide
assert "pwsh -NoProfile -File" not in operations_guide
assert "https://github.com/a3-suite/a3-rust-cruiser" in setup_guide
assert "/download/" not in setup_guide
assert 'powershell -ExecutionPolicy Bypass -c "irm ' not in setup_guide

for platform_id, (manifest_path, asset_name) in expected.items():
    declaration = assembly["platforms"][platform_id]
    assert declaration == {"manifest": manifest_path, "assetName": asset_name}
    manifest = json.loads((root / manifest_path).read_text(encoding="utf-8"))
    assert manifest["schemaVersion"] == "installer.manifest.v2"
    assert manifest["targetPlatformId"] == "<target-platform-id>"
    assert manifest["source"] == {
        "kind": "github-release",
        "owner": "<repository-owner>",
        "repository": "<repository-name>",
        "fixedReference": "<release-tag>",
    }
    assert manifest["placement"]["profile"] == "per-user-cli"
    assert manifest["placement"]["channel"] == "standalone"
    release_path = manifest["placement"]["releasePath"].rstrip("/\\").replace("\\", "/")
    assert release_path.rsplit("/", 1)[-1] == release_version
    assert manifest["activation"]["strategy"] == "active-pointer"
    assert manifest["compatibility"]["requiredInstallerVersion"] == "2"

assert not (root / ".ci/scripts/ci-release-supplemental-asset.sh").exists()
assert not (root / ".ci/scripts/ci-release-supplemental-asset.py").exists()
installer_files = {
    path.relative_to(root).as_posix()
    for path in (root / "installer").rglob("*")
    if path.is_file()
}
assert installer_files == {
    "installer/assembly.json",
    "installer/manifests/linux-x86_64.json",
    "installer/manifests/macos-arm64.json",
    "installer/manifests/windows-x86_64.json",
}, sorted(installer_files)
PY

echo "standard installer contract tests passed"
}

installer_standard_contract
