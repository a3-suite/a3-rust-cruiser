#!/usr/bin/env bash

set -euo pipefail

source_root="${1:?source checkout is required}"
target="${2:?target triple is required}"
tag_ref="${3:?tag ref is required}"
source_sha="${4:?source SHA is required}"
release_version="${5:?release version is required}"
output_dir="${6:?output directory is required}"
script_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
release_assets_contract="$script_root/../release-assets.json"

runner_label="$(jq -er --arg target "$target" '.platforms | map(select(.target == $target)) | if length == 1 then .[0].runner else error("unsupported release target") end' "$release_assets_contract")" || { echo "unsupported release target: $target" >&2; exit 1; }
[[ "$tag_ref" == "refs/tags/v$release_version" ]] || { echo "tag and version mismatch" >&2; exit 1; }
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo "source SHA must be a full commit SHA" >&2; exit 1; }
[[ "$(git -C "$source_root" rev-parse HEAD)" == "$source_sha" ]] || { echo "source checkout SHA mismatch" >&2; exit 1; }

bash "$script_root/verify-ci-tools.sh" "$runner_label"
toolchain_report="$(cd "$source_root" && rustup toolchain install --no-self-update && bash "$script_root/verify-rust-toolchain.sh" "$target")"
printf '%s\n' "$toolchain_report"

(
  cd "$source_root"
  host_target="$(rustc -vV | sed -n 's/^host: //p')"
  [[ "$host_target" == "$target" ]] || { echo "runner host $host_target does not match target $target" >&2; exit 1; }
  cargo fmt --all -- --check
  cargo clippy --locked --all-targets --all-features -- -D warnings
  cargo test --locked --all-targets --all-features
  cargo build --locked --release --target "$target"
  "target/$target/release/a3-rust-cruiser" check examples/minimal-project --policy examples/minimal-project/boundary-policy.yaml --package minimal-project --target bin:minimal-project --target-triple "$target" --format text | grep -F 'decision: conformant'
)

asset_name="a3-rust-cruiser-v${release_version}-${target}.tar.gz"
mkdir -p "$output_dir/package"
install -m 0755 "$source_root/target/$target/release/a3-rust-cruiser" "$output_dir/package/a3-rust-cruiser"
tar -C "$output_dir/package" -czf "$output_dir/$asset_name" a3-rust-cruiser
rm -r "$output_dir/package"
if command -v sha256sum >/dev/null; then asset_sha256="$(sha256sum "$output_dir/$asset_name" | awk '{print $1}')"; else asset_sha256="$(shasum -a 256 "$output_dir/$asset_name" | awk '{print $1}')"; fi
jq -n --arg asset_name "$asset_name" --arg asset_sha256 "$asset_sha256" --arg source_sha "$source_sha" --arg tag_ref "$tag_ref" --arg target "$target" --arg version "$release_version" '{schema_version: 1, product: "a3-rust-cruiser", version: $version, tag_ref: $tag_ref, source_sha: $source_sha, target: $target, asset_name: $asset_name, asset_sha256: $asset_sha256}' > "$output_dir/manifest.json"
[[ -s "$output_dir/$asset_name" ]] || { echo "release archive is empty" >&2; exit 1; }
[[ "$(tar -tzf "$output_dir/$asset_name")" == "a3-rust-cruiser" ]] || { echo "release archive has unexpected contents" >&2; exit 1; }
