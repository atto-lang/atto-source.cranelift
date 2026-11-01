#!/bin/sh
# Exact-head native Oreslang source -> Cranelift IR -> native JIT.
# No host-language arithmetic oracle may substitute for guest execution.
set -eu
export LC_ALL=C
cd "$(dirname "$0")/.."
die() { printf '%s\n' "CRANELIFT_NATIVE_NOT_ACCEPTED: $*" >&2; exit 1; }
actual_head="$(git rev-parse --verify HEAD)" || die "no source commit"
case "${EXPECTED_SOURCE_SHA:-}" in
  '') ;;
  *) [ "$actual_head" = "$EXPECTED_SOURCE_SHA" ] || die "source commit mismatch" ;;
esac
[ -z "$(git status --porcelain --untracked-files=normal -- tests/portable-scalar)" ] || die "dirty fixture tree"
compiler="${1:-target/release/orescranelift}"
[ -f "$compiler" ] && [ -x "$compiler" ] && [ ! -L "$compiler" ] || die "native Cranelift compiler absent"
limit="$(command -v timeout || command -v gtimeout || true)"
[ -n "$limit" ] || die "bounded runner unavailable"
tmp="$(mktemp -d)" || die "cannot allocate temp directory"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
chmod 700 "$tmp"
expected_count=0
: > "$tmp/seen"
while IFS='|' read -r path result || [ -n "$path" ]; do
  case "$path" in tests/portable-scalar/[0-9][0-9]-*.ores) ;; *) die "unrecognized test path $path";; esac
  [ "$result" = "42" ] || die "unexpected integer oracle for $path"
  [ -f "$path" ] && [ ! -L "$path" ] || die "nonregular guest fixture"
  git ls-files --error-unmatch -- "$path" >/dev/null || die "untracked guest fixture"
  grep -Fqx "$path" "$tmp/seen" && die "duplicate fixture"
  printf '%s\n' "$path" >> "$tmp/seen"
  expected_count=$((expected_count + 1))
  [ "$expected_count" -le 6 ] || die "too many cases"
  "$limit" -k 3s 20s "$compiler" ir "$path" > "$tmp/ir" 2> "$tmp/compile.err" || {
    cat "$tmp/compile.err" >&2
    die "native Cranelift IR compilation failed for $path"
  }
  [ -s "$tmp/ir" ] || die "compiler produced empty native IR"
  "$limit" -k 3s 20s "$compiler" run "$path" > "$tmp/output" 2> "$tmp/run.err" || {
    cat "$tmp/run.err" >&2
    die "native Cranelift execution failed for $path"
  }
  printf '%s\n' "$result" > "$tmp/expected"
  cmp -s "$tmp/expected" "$tmp/output" || die "wrong native output for $path"
  printf 'CRANELIFT_NATIVE_CASE_PASS path=%s git_blob=%s\n' "$path" "$(git rev-parse "HEAD:$path")"
done < tests/portable-scalar/oracles.tsv
[ "$expected_count" -eq 6 ] || die "missing expected scalar cases"
[ "$(git ls-files 'tests/portable-scalar/*.ores' | wc -l | tr -d ' ')" -eq 6 ] || die "unmanifested guest fixture"
printf 'CRANELIFT_NATIVE_ACCEPTED source_sha=%s cases=6\n' "$actual_head"
