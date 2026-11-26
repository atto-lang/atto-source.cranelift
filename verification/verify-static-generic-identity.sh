#!/usr/bin/env bash
set -Eeuo pipefail
compiler="$(realpath "$1")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"
cat > driver.c <<'EOF'
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
extern int32_t oreslang_scalar_entry(int64_t,int64_t,int64_t,int64_t,int64_t*);
int main(void) {
  int64_t out=-1;
  int32_t status=oreslang_scalar_entry(0,0,0,0,&out);
  if (status != 0) return 1;
  return printf("%" PRId64 "\n",out)<0;
}
EOF
pre='fnc identity<T>(T value): T { return value; }'
count=0
for expr in 'identity<int>(42)' 'identity<T=int>(42)' 'identity<T=int>(value: 42)' 'identity(42)' 'identity<T=int>(20)+identity<int>(22)'; do
  printf '%s pub fnc main(): int { return %s; }' "$pre" "$expr" > main.atto
  "$compiler" check main.atto
  test "$("$compiler" run main.atto)" = 42
  "$compiler" emit-object main.atto > main.o
  test -s main.o
  cc -std=c11 -Wall -Wextra -Werror driver.c main.o -o guest
  test "$(./guest)" = 42
  printf 'CRANELIFT_OBJECT_AND_JIT_PASS %s\n' "$expr"
  count=$((count+1))
done
for expr in 'identity<U=int>(42)' 'identity<T=3>(42)' 'identity<T=bool>(true)' 'identity<T=int,T=int>(42)' 'identity[T=int](42)' 'identity<T=int](42)' 'identity<T=int>(true)' 'identity<T=int>(wrong:42)' 'identity<T=int>(value:42,value:42)' 'identity<T=int>(value:)' 'identity<T=int>'; do
  printf '%s pub fnc main(): int { return %s; }' "$pre" "$expr" > main.atto
  for mode in check ir run emit-object; do
    status=0
    "$compiler" "$mode" main.atto > out 2> err || status=$?
    test "$status" -eq 1 && test ! -s out && test -s err
  done
  printf 'CRANELIFT_REJECT_PASS %s\n' "$expr"
  count=$((count+1))
done
test "$count" -eq 16
printf 'CRANELIFT_GENERIC_VERIFIED %s cases\n' "$count"

for source in \
 'pub fnc main(): int { if 1 < 2 do return 42; fi return 0; }' \
 'pub fnc main(): int { if 2 < 1 do return 0; else return 42; fi }' \
 'pub fnc main(): int { if true do return 42; else return 9223372036854775807+1; fi }' \
 'pub fnc main(): int { if false do return 7/0; fi return 42; }' \
 'pub fnc main(): int { if 1 > 2 do return 0; else return 42; fi }'; do
  printf '%s' "$source" > main.atto
  "$compiler" check main.atto
  test "$("$compiler" run main.atto)" = 42
  "$compiler" emit-object main.atto > main.o
  cc -std=c11 -Wall -Wextra -Werror driver.c main.o -o guest
  test "$(./guest)" = 42
  printf 'CRANELIFT_STATIC_CONDITION_PASS %s\n' "$source"
done
for source in \
 'pub fnc main(): int { if 1 do return 42; fi return 0; }' \
 'pub fnc main(): int { if true do return 42; else return missing; fi }' \
 'pub fnc main(): int { if true do return 42; }' \
 'pub fnc main(): int { if true do return 42; else return false; fi }' \
 'pub fnc main(x:int): int { if x < 2 do return 42; fi return 0; }'; do
  printf '%s' "$source" > main.atto
  status=0
  "$compiler" emit-object main.atto > out 2> err || status=$?
  test "$status" -eq 1 && test ! -s out && test -s err
  printf 'CRANELIFT_STATIC_CONDITION_REJECT %s\n' "$source"
done
printf 'CRANELIFT_STATIC_CONDITION_VERIFIED 10 cases\n'
