#!/usr/bin/env bash

set -euo pipefail

repository="a3-suite/a3-rust-cruiser"
version=""
install_dir="${HOME:?HOME is required}/.local/bin"
temporary_root=""
lock_dir=""
lock_acquired=false
archive=""
checksums=""
candidate=""

usage() {
  cat <<'EOF'
Usage: install-release.sh --version <VERSION> [--install-dir <DIRECTORY>]

Install a fixed a3-rust-cruiser GitHub Release after SHA-256 verification.

Options:
  --version <VERSION>        Required release version without a leading v
  --install-dir <DIRECTORY>  Existing absolute directory (default: $HOME/.local/bin)
  -h, --help                 Show this help
EOF
}

fail() {
  echo "a3-rust-cruiser installer: $*" >&2
  exit 1
}

cleanup() {
  local original_status=$?
  local cleanup_failed=false
  trap - EXIT
  for temporary_file in "$candidate" "$archive" "$checksums"; do
    [[ -n "$temporary_file" ]] || continue
    if [[ -f "$temporary_file" || -L "$temporary_file" ]]; then
      unlink "$temporary_file" || cleanup_failed=true
    fi
  done
  if [[ -n "$temporary_root" && -d "$temporary_root" && ! -L "$temporary_root" ]]; then
    rmdir "$temporary_root" || cleanup_failed=true
  fi
  if [[ "$lock_acquired" == true && -n "$lock_dir" && -d "$lock_dir" && ! -L "$lock_dir" ]]; then
    rmdir "$lock_dir" || cleanup_failed=true
  fi
  if [[ "$cleanup_failed" == true ]]; then
    echo "a3-rust-cruiser installer: failed to remove installer-owned temporary files" >&2
    [[ "$original_status" -ne 0 ]] || original_status=1
  fi
  exit "$original_status"
}

trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      [[ $# -ge 2 ]] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --install-dir)
      [[ $# -ge 2 ]] || fail "--install-dir requires a value"
      install_dir="$2"
      shift 2
      ;;
    --replace)
      fail "replacement is not supported; move the existing executable only after separate approval"
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
done

[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || fail "--version must be a complete release version such as 0.1.0"
[[ "$install_dir" == /* ]] || fail "install directory must be an absolute path"
[[ -d "$install_dir" && ! -L "$install_dir" ]] || fail "install directory must be an existing non-symlink directory: $install_dir"
[[ -w "$install_dir" ]] || fail "install directory is not writable: $install_dir"

for command_name in awk chmod cmp curl ln mkdir mktemp rmdir tar uname unlink; do
  command -v "$command_name" >/dev/null || fail "$command_name is required"
done
if command -v sha256sum >/dev/null; then
  checksum_command=(sha256sum)
elif command -v shasum >/dev/null; then
  checksum_command=(shasum -a 256)
else
  fail "sha256sum or shasum is required"
fi

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) target="x86_64-unknown-linux-gnu" ;;
  Darwin:arm64|Darwin:aarch64) target="aarch64-apple-darwin" ;;
  *) fail "unsupported platform: $(uname -s) $(uname -m)" ;;
esac

resolved_install_dir="$(cd "$install_dir" && pwd -P)"
path_contains_install_dir=false
IFS=: read -r -a path_entries <<< "${PATH:-}"
for path_entry in "${path_entries[@]}"; do
  [[ -n "$path_entry" ]] || path_entry="."
  [[ -d "$path_entry" ]] || continue
  resolved_path_entry="$(cd "$path_entry" && pwd -P)"
  if [[ "$resolved_path_entry" == "$resolved_install_dir" ]]; then
    path_contains_install_dir=true
    break
  fi
  if [[ -x "$resolved_path_entry/a3-rust-cruiser" ]]; then
    fail "a3-rust-cruiser is shadowed by $resolved_path_entry/a3-rust-cruiser"
  fi
done
[[ "$path_contains_install_dir" == true ]] || fail "install directory is not available on PATH: $resolved_install_dir"

cd "$resolved_install_dir"
[[ "$(pwd -P)" == "$resolved_install_dir" ]] || fail "install directory identity changed before activation: $resolved_install_dir"
destination="./a3-rust-cruiser"
destination_display="$resolved_install_dir/a3-rust-cruiser"
destination_initial_state="absent"
destination_initial_checksum=""
if [[ -e "$destination" || -L "$destination" ]]; then
  [[ -f "$destination" && ! -L "$destination" ]] || fail "existing destination is not a regular file: $destination"
  destination_initial_state="present"
  destination_initial_checksum="$("${checksum_command[@]}" "$destination" | awk '{print $1}')"
fi
lock_dir="./.a3-rust-cruiser-install.lock"
mkdir "$lock_dir" 2>/dev/null || fail "another install may be active: $lock_dir"
lock_acquired=true
temporary_root="$(mktemp -d "./.a3-rust-cruiser-install.XXXXXX")" || fail "failed to create temporary directory"
[[ -d "$temporary_root" && ! -L "$temporary_root" ]] || fail "invalid temporary directory"

asset_name="a3-rust-cruiser-v${version}-${target}.tar.gz"
release_base="https://github.com/${repository}/releases/download/v${version}"
archive="$temporary_root/$asset_name"
checksums="$temporary_root/SHA256SUMS"

curl --fail --location --proto '=https' --tlsv1.2 --output "$checksums" "$release_base/SHA256SUMS"
curl --fail --location --proto '=https' --tlsv1.2 --output "$archive" "$release_base/$asset_name"

checksum_matches="$(awk -v asset="$asset_name" '$2 == asset { count += 1; value = $1 } END { if (count == 1) print value }' "$checksums")"
[[ "$checksum_matches" =~ ^[0-9a-f]{64}$ ]] || fail "SHA256SUMS does not contain exactly one valid checksum for $asset_name"
actual_checksum="$("${checksum_command[@]}" "$archive" | awk '{print $1}')"
[[ "$actual_checksum" == "$checksum_matches" ]] || fail "release asset checksum mismatch"

archive_entries="$(tar -tzf "$archive")"
[[ "$archive_entries" == "a3-rust-cruiser" ]] || fail "release archive must contain only a3-rust-cruiser"
archive_entry_details="$(tar -tvzf "$archive")"
[[ "${archive_entry_details:0:1}" == "-" ]] || fail "release archive entry must be a regular file"

candidate="$temporary_root/a3-rust-cruiser"
tar -xOzf "$archive" a3-rust-cruiser > "$candidate"
chmod 0755 "$candidate"
candidate_version="$($candidate --version)" || fail "release executable version check failed"
[[ "$candidate_version" == "a3-rust-cruiser $version" ]] || fail "release executable version mismatch: $candidate_version"

verify_destination_unchanged() {
  if [[ "$destination_initial_state" == "absent" ]]; then
    [[ ! -e "$destination" && ! -L "$destination" ]] || fail "destination changed during install: $destination_display"
  else
    [[ -f "$destination" && ! -L "$destination" ]] || fail "destination changed during install: $destination_display"
    destination_current_checksum="$("${checksum_command[@]}" "$destination" | awk '{print $1}')"
    [[ "$destination_current_checksum" == "$destination_initial_checksum" ]] || fail "destination changed during install: $destination_display"
  fi
}

verify_destination_unchanged

if [[ -e "$destination" ]] && cmp -s "$candidate" "$destination"; then
  printf 'Already installed: %s\n' "$destination_display"
elif [[ -e "$destination" || -L "$destination" ]]; then
  fail "a different executable already exists; replacement is not supported: $destination_display"
else
  verify_destination_unchanged
  if ! ln "$candidate" "$destination"; then
    if [[ ! -f "$destination" || -L "$destination" || ! "$destination" -ef "$candidate" ]]; then
      fail "destination changed before activation: $destination_display"
    fi
  fi
  printf 'Installed executable: %s\n' "$destination_display"
fi

if [[ ! "$resolved_install_dir" -ef . ]]; then
  fail "install directory changed after activation; executable retained at $(pwd -P)/a3-rust-cruiser"
fi
installed_version="$($destination --version)" || fail "installed executable version check failed"
[[ "$installed_version" == "a3-rust-cruiser $version" ]] || fail "installed executable version mismatch: $installed_version"
hash -r
resolved_command="$(command -v a3-rust-cruiser || true)"
[[ -n "$resolved_command" && "$resolved_command" -ef "$destination" ]] || fail "installed command does not resolve to $destination_display"
printf 'Installed version: %s\n' "$version"
printf 'Installed command: %s\n' "$resolved_command"
