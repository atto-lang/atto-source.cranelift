#!/bin/sh
# Native Cranelift scalar guest gate: reviewed source Git blobs -> real IR -> JIT.
# A source inventory or mock compiler is not native Oreslang evidence.
set -eu
export LC_ALL=C
cd "$(dirname "$0")/.."
die() { printf '%s\n' "CRANELIFT_NATIVE_NOT_ACCEPTED: $*" >&2; exit 1; }
case "${EXPECTED_SOURCE_SHA:-}" in
  ''|*[!0-9a-f]*) die "missing or invalid expected exact-head SHA";;
esac
[ "${#EXPECTED_SOURCE_SHA}" -eq 40 ] || die "source SHA must be 40 hex digits"
actual_head="$(git rev-parse --verify HEAD)" || die "source commit missing"
[ "$actual_head" = "$EXPECTED_SOURCE_SHA" ] || die "source commit mismatch"
[ -d tests/portable-scalar ] && [ ! -L tests/portable-scalar ] || die "fixture directory invalid"
[ -f tests/portable-scalar/oracles.tsv ] && [ ! -L tests/portable-scalar/oracles.tsv ] || die "invalid oracle manifest"
[ -z "$(git status --porcelain --untracked-files=normal -- tests/portable-scalar)" ] || die "dirty source fixtures"
compiler="${1:-target/debug/orescranelift}"
[ -f "$compiler" ] && [ -x "$compiler" ] && [ ! -L "$compiler" ] || die "native compiler must be a regular executable"
case "${CRANELIFT_COMPILER_SHA256:-}" in
  ''|*[!0-9a-f]*) die "missing or invalid compiler SHA-256";;
esac
[ "${#CRANELIFT_COMPILER_SHA256}" -eq 64 ] || die "compiler digest must be 64 hex digits"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum unavailable"
fingerprint() { sha256sum "$compiler" | cut -d ' ' -f 1; }
[ "$(fingerprint)" = "$CRANELIFT_COMPILER_SHA256" ] || die "compiler identity mismatch"
limit="$(command -v timeout || command -v gtimeout || true)"
[ -n "$limit" ] || die "bounded runner unavailable"
tmp="$(mktemp -d)" || die "cannot allocate temp directory"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
chmod 700 "$tmp"
count=0
: > "$tmp/seen"
while IFS='|' read -r path result blob extra || [ -n "$path" ]; do
  [ -z "$extra" ] || die "unexpected oracle columns for $path"
  case "$path" in tests/portable-scalar/[0-9][0-9]-*.ores) ;; *) die "invalid test path $path";; esac
  [ "$result" = 42 ] || die "unknown numeric oracle for $path"
  case "$blob" in
    ''|*[!0-9a-f]*) die "invalid pinned source blob for $path";;
  esac
  [ "${#blob}" -eq 40 ] || die "invalid source blob width for $path"
  [ -f "$path" ] && [ ! -L "$path" ] || die "source must be a regular file $path"
  git ls-files --error-unmatch -- "$path" >/dev/null || die "untracked source: $path"
  grep -Fqx "$path" "$tmp/seen" && die "duplicate source: $path"
  printf '%s\n' "$path" >> "$tmp/seen"
  count=$((count + 1))
  [ "$count" -le 6 ] || die "too many cases"
  reviewed_blob="$(git rev-parse "HEAD:$path")" || die "source blob missing from revision"
  [ "$reviewed_blob" = "$blob" ] || die "manifest-to-tree blob mismatch $path"
  [ "$(git hash-object -- "$path")" = "$blob" ] || die "on-disk source differs from exact Git tree"
  "$limit" -k 3s 20s "$compiler" ir "$path" > "$tmp/ir" 2> "$tmp/compile.err" || {
    cat "$tmp/compile.err" >&2
    die "Cranelift IR compilation failed: $path"
  }
  [ -s "$tmp/ir" ] && grep -Fq "function" "$tmp/ir" || die "missing native Cranelift IR"
  [ ! -s "$tmp/compile.err" ] || die "unexpected compiler diagnostic for $path"
  "$limit" -k 3s 20s "$compiler" run "$path" > "$tmp/output" 2> "$tmp/run.err" || {
    cat "$tmp/run.err" >&2
    die "Cranelift guest JIT execution failed: $path"
  }
  [ ! -s "$tmp/run.err" ] || die "unexpected guest stderr for $path"
  printf '%s\n' "$result" > "$tmp/expected"
  cmp -s "$tmp/expected" "$tmp/output" || die "wrong native result for $path"
  [ "$(git hash-object -- "$path")" = "$blob" ] || die "fixture mutated during compilation: $path"
  [ "$(fingerprint)" = "$CRANELIFT_COMPILER_SHA256" ] || die "compiler mutated during execution"
  printf 'CRANELIFT_NATIVE_CASE_PASS path=%s git_blob=%s\n' "$path" "$blob"
done < tests/portable-scalar/oracles.tsv
[ "$count" -eq 6 ] || die "missing scalar cases"
[ "$(git ls-files 'tests/portable-scalar/*.ores' | wc -l | tr -d ' ')" -eq 6 ] || die "unmanifested source"
[ -z "$(git status --porcelain --untracked-files=normal -- tests/portable-scalar)" ] || die "source tree mutated"
[ "$(fingerprint)" = "$CRANELIFT_COMPILER_SHA256" ] || die "compiler changed after all cases"
printf 'CRANELIFT_NATIVE_ACCEPTED source_sha=%s compiler_sha256=%s cases=6\n' "$actual_head" "$CRANELIFT_COMPILER_SHA256"
