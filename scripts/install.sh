#!/usr/bin/env bash
set -euo pipefail
# ACS_RELEASE_DIR supports offline installation from an already downloaded release.
asset=acs-cli-linux-x64-musl
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || { printf 'Unsupported platform; Linux x86_64 is required\n' >&2; exit 1; }
for tool in sha256sum od mktemp; do command -v "$tool" >/dev/null || { printf 'Missing tool: %s\n' "$tool" >&2; exit 1; }; done
tmp_parent="$(cd "${TMPDIR:-/tmp}" && pwd -P)"
task_tmp="$(mktemp -d "$tmp_parent/acs-install-XXXXXXXX")"
[[ "$task_tmp" == "$tmp_parent"/acs-install-* && -d "$task_tmp" ]] || exit 1
trap 'rm -rf -- "$task_tmp"' EXIT
base="${ACS_RELEASE_BASE:-https://github.com/shellus/acs/releases/latest/download}"
fetch() {
  if [[ -n "${ACS_RELEASE_DIR:-}" ]]; then cp -- "$ACS_RELEASE_DIR/$1" "$2";
  else command -v curl >/dev/null; curl --fail --location --silent --show-error --connect-timeout 15 --speed-limit 1024 --speed-time 60 "$base/$1" -o "$2"; fi
}
fetch "$asset" "$task_tmp/$asset"
fetch "$asset.sha256" "$task_tmp/sum"
# Match exactly one named checksum, never accept an empty pipeline as success.
expected="$(awk -v name="$asset" '$2 == name || $2 == "*" name {print $1}' "$task_tmp/sum")"
[[ "$expected" =~ ^[[:xdigit:]]{64}$ ]] || { printf 'Missing, duplicate or invalid SHA-256\n' >&2; exit 1; }
actual="$(sha256sum "$task_tmp/$asset")"; actual="${actual%% *}"
expected="$(printf %s "$expected" | tr A-F a-f)"
[[ "$expected" == "$actual" ]] || { printf 'SHA-256 mismatch; installation unchanged\n' >&2; exit 1; }
file="$task_tmp/$asset"
magic="$(od -An -tx1 -N6 "$file" | tr -d ' \n')"
[[ "$magic" == 7f454c460201 && "$(od -An -tu2 -j18 -N2 "$file" | tr -d ' ')" == 62 ]] || { printf 'Not Linux x64 ELF\n' >&2; exit 1; }
phoff="$(od -An -tu8 -j32 -N8 "$file" | tr -d ' ')"
phsize="$(od -An -tu2 -j54 -N2 "$file" | tr -d ' ')"
phcount="$(od -An -tu2 -j56 -N2 "$file" | tr -d ' ')"
[[ "$phoff" =~ ^[0-9]+$ && "$phsize" =~ ^[0-9]+$ && "$phcount" =~ ^[0-9]+$ ]] || exit 1
(( phcount == 0 || phsize >= 56 )) || exit 1
for ((i=0; i<phcount; i++)); do
  type="$(od -An -tu4 -j"$((phoff+i*phsize))" -N4 "$file" | tr -d ' ')"
  [[ "$type" =~ ^[0-9]+$ && "$type" != 3 ]] || { printf 'Dynamic or invalid ELF; static musl CLI required\n' >&2; exit 1; }
done
target_dir="${ACS_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p -- "$target_dir"
# Same-directory rename ensures an interrupted write never truncates the installed CLI.
staged="$(mktemp "$target_dir/.acs-install-XXXXXX")"
trap 'rm -f -- "$staged"; rm -rf -- "$task_tmp"' EXIT
cp -- "$file" "$staged"; chmod 0755 "$staged"; mv -f -- "$staged" "$target_dir/acs"
printf 'Installed acs CLI to %s/acs; add this directory to PATH if necessary.\n' "$target_dir"
