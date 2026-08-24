#!/usr/bin/env bash

set -euo pipefail

skill_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
installer="$skill_root/scripts/install-release.sh"
command -v python3 >/dev/null || { echo "python3 is required for test cleanup" >&2; exit 1; }
test_root="$(mktemp -d "${TMPDIR:-/tmp}/a3-rust-cruiser-installer-test.XXXXXX")"

cleanup() {
  local original_status=$?
  trap - EXIT
  if [[ -d "$test_root" && ! -L "$test_root" ]]; then
    python3 -c 'import pathlib, shutil, sys
path = pathlib.Path(sys.argv[1])
if path.is_symlink() or not path.is_dir():
    raise SystemExit("test root identity check failed")
shutil.rmtree(path)' "$test_root"
  fi
  exit "$original_status"
}
trap cleanup EXIT

fixture_root="$test_root/release"
fake_bin="$test_root/fake-bin"
mkdir -p "$fixture_root" "$fake_bin"

payload="$test_root/a3-rust-cruiser"
asset_name="a3-rust-cruiser-v0.1.0-aarch64-apple-darwin.tar.gz"

write_payload() {
  local payload_version="$1"
  cat > "$payload" <<EOF
#!/usr/bin/env bash
if [[ "\${1:-}" == "--version" ]]; then
  echo "a3-rust-cruiser $payload_version"
  exit 0
fi
exit 3
EOF
  chmod 0755 "$payload"
}

refresh_checksum() {
  if command -v sha256sum >/dev/null; then
    asset_checksum="$(sha256sum "$fixture_root/$asset_name" | awk '{print $1}')"
  else
    asset_checksum="$(shasum -a 256 "$fixture_root/$asset_name" | awk '{print $1}')"
  fi
  printf '%s  %s\n' "$asset_checksum" "$asset_name" > "$fixture_root/SHA256SUMS"
}

build_payload_archive() {
  tar -C "$test_root" -czf "$fixture_root/$asset_name" a3-rust-cruiser
  refresh_checksum
}

write_payload 0.1.0
build_payload_archive

cat > "$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --fail|--location|--tlsv1.2) shift ;;
    --proto) shift 2 ;;
    *) url="$1"; shift ;;
  esac
done
cp "$A3_RUST_CRUISER_TEST_RELEASE_ROOT/${url##*/}" "$output"
if [[ "${url##*/}" != "SHA256SUMS" && -n "${A3_RUST_CRUISER_TEST_MUTATE_DESTINATION:-}" ]]; then
  printf '#!/usr/bin/env bash\necho changed\n' > "$A3_RUST_CRUISER_TEST_MUTATE_DESTINATION"
  chmod 0755 "$A3_RUST_CRUISER_TEST_MUTATE_DESTINATION"
fi
EOF
chmod 0755 "$fake_bin/curl"

cat > "$fake_bin/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) echo Darwin ;;
  -m) echo arm64 ;;
  *) echo Darwin ;;
esac
EOF
chmod 0755 "$fake_bin/uname"

run_installer() {
  local install_dir="$1"
  shift
  PATH="$install_dir:$fake_bin:$PATH" \
    A3_RUST_CRUISER_TEST_RELEASE_ROOT="$fixture_root" \
    bash "$installer" --version 0.1.0 --install-dir "$install_dir" "$@"
}

unsupported_bin="$test_root/unsupported-bin"
unsupported_dir="$test_root/unsupported"
mkdir "$unsupported_bin" "$unsupported_dir"
cat > "$unsupported_bin/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) echo Linux ;;
  -m) echo arm64 ;;
  *) echo Linux ;;
esac
EOF
chmod 0755 "$unsupported_bin/uname"
if PATH="$unsupported_dir:$unsupported_bin:$fake_bin:$PATH" \
  A3_RUST_CRUISER_TEST_RELEASE_ROOT="$fixture_root" \
  bash "$installer" --version 0.1.0 --install-dir "$unsupported_dir" \
  >"$test_root/unsupported.out" 2>"$test_root/unsupported.err"; then
  echo "installer unexpectedly accepted an unsupported platform" >&2
  exit 1
fi
grep -F 'unsupported platform:' "$test_root/unsupported.err" >/dev/null

semver_dir="$test_root/semver"
mkdir "$semver_dir"
for invalid_version in 01.0.0 1.01.0 1.0.00; do
  if PATH="$semver_dir:$fake_bin:$PATH" bash "$installer" --version "$invalid_version" --install-dir "$semver_dir" \
    >"$test_root/semver.out" 2>"$test_root/semver.err"; then
    echo "installer unexpectedly accepted invalid SemVer: $invalid_version" >&2
    exit 1
  fi
  grep -F 'must be a complete release version' "$test_root/semver.err" >/dev/null
done

install_dir="$test_root/install"
mkdir "$install_dir"
run_installer "$install_dir"
[[ "$("$install_dir"/a3-rust-cruiser --version)" == "a3-rust-cruiser 0.1.0" ]]
run_installer "$install_dir" | grep -F 'Already installed:' >/dev/null

locked_dir="$test_root/locked"
mkdir "$locked_dir" "$locked_dir/.a3-rust-cruiser-install.lock"
if run_installer "$locked_dir" >"$test_root/locked.out" 2>"$test_root/locked.err"; then
  echo "installer unexpectedly ignored an active install lock" >&2
  exit 1
fi
grep -F 'another install may be active:' "$test_root/locked.err" >/dev/null
[[ -d "$locked_dir/.a3-rust-cruiser-install.lock" ]]

protected_dir="$test_root/protected"
mkdir "$protected_dir"
printf '#!/usr/bin/env bash\necho old\n' > "$protected_dir/a3-rust-cruiser"
chmod 0755 "$protected_dir/a3-rust-cruiser"
if run_installer "$protected_dir" >"$test_root/protected.out" 2>"$test_root/protected.err"; then
  echo "installer unexpectedly replaced an existing executable" >&2
  exit 1
fi
grep -F 'replacement is not supported' "$test_root/protected.err" >/dev/null
[[ "$("$protected_dir"/a3-rust-cruiser)" == "old" ]]
if run_installer "$protected_dir" --replace >"$test_root/replace.out" 2>"$test_root/replace.err"; then
  echo "installer unexpectedly accepted destructive replacement" >&2
  exit 1
fi
grep -F 'replacement is not supported' "$test_root/replace.err" >/dev/null
[[ "$("$protected_dir"/a3-rust-cruiser)" == "old" ]]

effect_then_fail_ln_bin="$test_root/effect-then-fail-ln-bin"
effect_then_fail_absent_dir="$test_root/effect-then-fail-absent"
mkdir "$effect_then_fail_ln_bin" "$effect_then_fail_absent_dir"
cat > "$effect_then_fail_ln_bin/ln" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
/bin/ln "$@"
exit 1
EOF
chmod 0755 "$effect_then_fail_ln_bin/ln"
if ! PATH="$effect_then_fail_absent_dir:$effect_then_fail_ln_bin:$fake_bin:$PATH" \
  A3_RUST_CRUISER_TEST_RELEASE_ROOT="$fixture_root" \
  bash "$installer" --version 0.1.0 --install-dir "$effect_then_fail_absent_dir" \
  >"$test_root/effect-then-fail-absent.out" 2>"$test_root/effect-then-fail-absent.err"; then
  echo "installer did not reconcile a completed no-clobber activation" >&2
  exit 1
fi
[[ "$("$effect_then_fail_absent_dir"/a3-rust-cruiser --version)" == "a3-rust-cruiser 0.1.0" ]]

invalid_dir="$test_root/invalid"
mkdir "$invalid_dir"
printf '%064d  %s\n' 0 "$asset_name" > "$fixture_root/SHA256SUMS"
if run_installer "$invalid_dir" >"$test_root/invalid.out" 2>"$test_root/invalid.err"; then
  echo "installer unexpectedly accepted a checksum mismatch" >&2
  exit 1
fi
grep -F 'release asset checksum mismatch' "$test_root/invalid.err" >/dev/null
[[ ! -e "$invalid_dir/a3-rust-cruiser" ]]

write_payload 0.1.0
printf 'unexpected\n' > "$test_root/unexpected"
tar -C "$test_root" -czf "$fixture_root/$asset_name" a3-rust-cruiser unexpected
refresh_checksum
invalid_archive_dir="$test_root/invalid-archive"
mkdir "$invalid_archive_dir"
if run_installer "$invalid_archive_dir" >"$test_root/invalid-archive.out" 2>"$test_root/invalid-archive.err"; then
  echo "installer unexpectedly accepted an archive with extra content" >&2
  exit 1
fi
grep -F 'release archive must contain only' "$test_root/invalid-archive.err" >/dev/null
[[ ! -e "$invalid_archive_dir/a3-rust-cruiser" ]]

unlink "$payload"
ln -s unexpected "$payload"
build_payload_archive
nonregular_dir="$test_root/nonregular"
mkdir "$nonregular_dir"
if run_installer "$nonregular_dir" >"$test_root/nonregular.out" 2>"$test_root/nonregular.err"; then
  echo "installer unexpectedly accepted a non-regular archive entry" >&2
  exit 1
fi
grep -F 'release archive entry must be a regular file' "$test_root/nonregular.err" >/dev/null
[[ ! -e "$nonregular_dir/a3-rust-cruiser" ]]

unlink "$payload"
write_payload 9.9.9
build_payload_archive
version_mismatch_dir="$test_root/version-mismatch"
mkdir "$version_mismatch_dir"
if run_installer "$version_mismatch_dir" >"$test_root/version-mismatch.out" 2>"$test_root/version-mismatch.err"; then
  echo "installer unexpectedly accepted a mismatched executable version" >&2
  exit 1
fi
grep -F 'release executable version mismatch:' "$test_root/version-mismatch.err" >/dev/null
[[ ! -e "$version_mismatch_dir/a3-rust-cruiser" ]]

write_payload 0.1.0
build_payload_archive

shadow_dir="$test_root/shadow"
shadow_install_dir="$test_root/shadow-install"
mkdir "$shadow_dir" "$shadow_install_dir"
printf '#!/usr/bin/env bash\necho shadow\n' > "$shadow_dir/a3-rust-cruiser"
chmod 0755 "$shadow_dir/a3-rust-cruiser"
if PATH="$shadow_dir:$shadow_install_dir:$fake_bin:$PATH" \
  A3_RUST_CRUISER_TEST_RELEASE_ROOT="$fixture_root" \
  bash "$installer" --version 0.1.0 --install-dir "$shadow_install_dir" \
  >"$test_root/shadow.out" 2>"$test_root/shadow.err"; then
  echo "installer unexpectedly accepted a shadowed command" >&2
  exit 1
fi
grep -F 'a3-rust-cruiser is shadowed by' "$test_root/shadow.err" >/dev/null
[[ ! -e "$shadow_install_dir/a3-rust-cruiser" ]]

competing_ln_bin="$test_root/competing-ln-bin"
no_clobber_dir="$test_root/no-clobber"
mkdir "$competing_ln_bin" "$no_clobber_dir"
cat > "$competing_ln_bin/ln" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
destination="${!#}"
printf '#!/usr/bin/env bash\necho competitor\n' > "$destination"
chmod 0755 "$destination"
exec /bin/ln "$@"
EOF
chmod 0755 "$competing_ln_bin/ln"
if PATH="$no_clobber_dir:$competing_ln_bin:$fake_bin:$PATH" \
  A3_RUST_CRUISER_TEST_RELEASE_ROOT="$fixture_root" \
  bash "$installer" --version 0.1.0 --install-dir "$no_clobber_dir" \
  >"$test_root/no-clobber.out" 2>"$test_root/no-clobber.err"; then
  echo "installer unexpectedly overwrote a destination created immediately before activation" >&2
  exit 1
fi
grep -F 'destination changed before activation:' "$test_root/no-clobber.err" >/dev/null
[[ "$("$no_clobber_dir"/a3-rust-cruiser)" == "competitor" ]]

directory_swap_ln_bin="$test_root/directory-swap-ln-bin"
directory_swap_dir="$test_root/directory-swap"
mkdir "$directory_swap_ln_bin" "$directory_swap_dir"
cat > "$directory_swap_ln_bin/ln" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
/bin/mv "$A3_RUST_CRUISER_TEST_SWAP_INSTALL_DIR" "$A3_RUST_CRUISER_TEST_SWAP_INSTALL_DIR.moved"
/bin/mkdir "$A3_RUST_CRUISER_TEST_SWAP_INSTALL_DIR"
exec /bin/ln "$@"
EOF
chmod 0755 "$directory_swap_ln_bin/ln"
if PATH="$directory_swap_dir:$directory_swap_ln_bin:$fake_bin:$PATH" \
  A3_RUST_CRUISER_TEST_RELEASE_ROOT="$fixture_root" \
  A3_RUST_CRUISER_TEST_SWAP_INSTALL_DIR="$directory_swap_dir" \
  bash "$installer" --version 0.1.0 --install-dir "$directory_swap_dir" \
  >"$test_root/directory-swap.out" 2>"$test_root/directory-swap.err"; then
  echo "installer unexpectedly accepted a replaced install directory" >&2
  exit 1
fi
grep -F 'install directory changed after activation; executable retained at' "$test_root/directory-swap.err" >/dev/null
[[ ! -e "$directory_swap_dir/a3-rust-cruiser" ]]
[[ "$("$directory_swap_dir".moved/a3-rust-cruiser --version)" == "a3-rust-cruiser 0.1.0" ]]

echo "install-release tests passed"
