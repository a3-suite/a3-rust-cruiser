#!/usr/bin/env bash

set -euo pipefail

toolchain_file="rust-toolchain.toml"
[[ -f "$toolchain_file" && ! -L "$toolchain_file" ]] || {
  echo "missing regular rust-toolchain.toml" >&2
  exit 1
}

toolchain_channel="$(awk -F'"' '/^[[:space:]]*channel[[:space:]]*=/ { print $2; exit }' "$toolchain_file")"
[[ "$toolchain_channel" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "rust-toolchain.toml must pin a semantic-version channel" >&2
  exit 1
}

active_toolchain="$(rustup show active-toolchain | awk 'NR == 1 { print $1; exit }')"
case "$active_toolchain" in
  "$toolchain_channel"|"$toolchain_channel"-*) ;;
  *)
    echo "active Rust toolchain does not match rust-toolchain.toml: expected $toolchain_channel, got ${active_toolchain:-<none>}" >&2
    exit 1
    ;;
esac

rustc_info="$(rustc -vV)"
rustc_release="$(sed -n 's/^release: //p' <<< "$rustc_info")"
rustc_host="$(sed -n 's/^host: //p' <<< "$rustc_info")"
rustc_commit_hash="$(sed -n 's/^commit-hash: //p' <<< "$rustc_info")"
llvm_version="$(sed -n 's/^LLVM version: //p' <<< "$rustc_info")"
cargo_version="$(cargo -V | sed -n 's/^cargo \([^ ]*\).*$/\1/p')"

[[ "$rustc_release" == "$toolchain_channel" ]] || {
  echo "rustc release does not match rust-toolchain.toml: expected $toolchain_channel, got ${rustc_release:-<none>}" >&2
  exit 1
}
[[ "$cargo_version" == "$toolchain_channel" ]] || {
  echo "cargo version does not match rust-toolchain.toml: expected $toolchain_channel, got ${cargo_version:-<none>}" >&2
  exit 1
}
[[ -n "$rustc_host" && -n "$rustc_commit_hash" && -n "$llvm_version" ]] || {
  echo "rustc -vV did not provide complete toolchain identity" >&2
  exit 1
}

targets=("$@")
if [[ "${#targets[@]}" -eq 0 ]]; then
  targets=("$rustc_host")
fi
for target in "${targets[@]}"; do
  [[ -n "$target" ]] || { echo "target must not be empty" >&2; exit 1; }
  rustc --print cfg --target "$target" >/dev/null || {
    echo "rustc cannot resolve target cfg: $target" >&2
    exit 1
  }
done

printf 'toolchain_channel=%s\n' "$toolchain_channel"
printf 'active_toolchain=%s\n' "$active_toolchain"
printf 'rustc_release=%s\n' "$rustc_release"
printf 'rustc_host=%s\n' "$rustc_host"
printf 'rustc_commit_hash=%s\n' "$rustc_commit_hash"
printf 'llvm_version=%s\n' "$llvm_version"
printf 'cargo_version=%s\n' "$cargo_version"
for target in "${targets[@]}"; do
  printf 'verified_target=%s\n' "$target"
done
