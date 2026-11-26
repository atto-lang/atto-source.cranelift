#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
compiler="$(realpath "$1")"
corpus="$(realpath "$2")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat > "$work/main.c" <<'C'
#include <stdint.h>
#include <stdio.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void){int64_t value=0;int32_t status=oreslang_scalar_entry(0,0,0,0,&value);if(status){fprintf(stderr,"status=%d\n",status);return 2;}printf("%lld\n",(long long)value);return 0;}
C
python3 - "$corpus" <<'PY'
import json,sys,pathlib,hashlib
root=pathlib.Path(sys.argv[1])
cases=[c for c in json.loads((root/'manifest.json').read_text())['cases'] if c['id'].startswith(('counted-','reject-counted-'))]
assert len(cases)==17
for c in cases:
    raw=(root/c['path']).read_bytes()
    assert hashlib.sha1(b'blob '+str(len(raw)).encode()+b'\0'+raw).hexdigest()==c['gitBlobSha1']
PY
for name in counted-three counted-zero counted-one counted-alias counted-mixed-future-alias counted-multi-channel-fair counted-mixed-zero counted-distinct-alias-projection; do
  "$compiler" emit-counted-static-ready-object "$corpus/$name.atto" > "$work/guest.o"
  file "$work/guest.o" | grep -F relocatable
  cc "$work/main.c" "$work/guest.o" -o "$work/guest"
  test "$("$work/guest")" = 42
  echo "CRANELIFT_NATIVE_GUEST_PASS $name"
done
for name in counted-dynamic counted-dynamic-zero reject-counted-negative reject-counted-bool reject-counted-scalar reject-counted-default reject-counted-write reject-counted-dynamic-bool reject-counted-dynamic-unknown; do
  if "$compiler" emit-counted-static-ready-object "$corpus/$name.atto" > "$work/denial" 2> "$work/err"; then exit 1; fi
  test ! -s "$work/denial"
  grep -F E_COUNTED_STATIC_READY "$work/err"
done
# Prove payload and projection failures execute in the generated native object,
# rather than being evaluated by the compiler or replaced with a constant 42.
for case in write-overflow projection-overflow; do
  case "$case" in
    write-overflow) body='writech a, 9223372036854775807 + 1; val Array[int] b = await select<int>(0) { when readch a: int n -> { return n; } }; return 42;';;
    projection-overflow) body='writech a, 9223372036854775807; val Array[int] b = await select<int>(1) { when readch a: int n -> { return n + 1; } }; return b[0];';;
  esac
  printf 'suspends fnc main(): int { val Channel<int> a = Channel.new<int>(1); %s }\n' "$body" > "$work/$case.atto"
  "$compiler" emit-counted-static-ready-object "$work/$case.atto" > "$work/guest.o"
  cc "$work/main.c" "$work/guest.o" -o "$work/guest"
  status=0
  "$work/guest" > "$work/out" 2> "$work/err" || status=$?
  test "$status" = 2; test ! -s "$work/out"; grep -F status=1 "$work/err"
  echo "CRANELIFT_RUNTIME_OVERFLOW_PASS $case"
done
# Unselected arm projection and zero count must not execute overflowing code.
for case in zero-projection losing-alias; do
  case "$case" in
    zero-projection) body='writech a, 42; val Array[int] b = await select<int>(0) { when readch a: int n -> { return n + 9223372036854775807; } }; return readch a;';;
    losing-alias) body='val Future<int> f = select<int> { default: -> { return 42; } }; val Array[int] b = await select<int>(1) fair over when await f: int n -> { return n; } when await f: int n -> { return n + 9223372036854775807; } end; return b[0];';;
  esac
  printf 'suspends fnc main(): int { val Channel<int> a = Channel.new<int>(1); %s }\n' "$body" > "$work/$case.atto"
  "$compiler" emit-counted-static-ready-object "$work/$case.atto" > "$work/guest.o"
  cc "$work/main.c" "$work/guest.o" -o "$work/guest"
  test "$("$work/guest")" = 42
  echo "CRANELIFT_NATIVE_CONTROL_PASS $case"
done
# Affine handle names remain tombstones after consumption; branch effects
# and competing access must reject rather than silently alter static topology.
for body in \
 'writech a, 42; val Future<Array<int>> f = select<int>(1) { when readch a: int n -> { return n; } }; return 42;' \
 'writech a, 42; val Future<Array<int>> f = select<int>(1) { when readch a: int n -> { return n; } }; val Array[int] b = await f; val int f = 42; return f;' \
 'writech a, 42; val Future<Array<int>> f = select<int>(1) { when readch a: int n -> { return n; } }; writech a, 5; val Array[int] b = await f; return b[0];' \
 'writech a, 42; val Array[int] b = await select<int>(0) { when readch a: int n -> { return n; } }; if (1 == 1) || ((readch a) == 42) then return 42; fi return 0;'; do
  printf 'suspends fnc main(): int { val Channel<int> a = Channel.new<int>(1); %s }\n' "$body" > "$work/bad.atto"
  if "$compiler" emit-counted-static-ready-object "$work/bad.atto" > "$work/denial" 2> "$work/err"; then exit 1; fi
  test ! -s "$work/denial"; grep -F E_COUNTED_STATIC_READY "$work/err"
done
echo CRANELIFT_COUNTED_STATIC_NATIVE_PASS
