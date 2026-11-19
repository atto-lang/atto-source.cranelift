#!/usr/bin/env bash
# Source-free adversarial tests; never claim compiler/native guest coverage.
set -euo pipefail
verify="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/verify.sh"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
cd "$work"
git init -q -b main
git config user.name 'Oreslang Audit Fixture'
git config user.email 'audit@example.invalid'
printf 'tracked\n' > tracked.txt
printf '*.cache\n' > .gitignore
git add tracked.txt .gitignore
git commit -qm 'fixture'
head="$(git rev-parse HEAD)"
pass=0
reject() {
  if bash "$verify" "$head" >/dev/null 2>&1; then
    echo "FAIL: $1 was admitted" >&2
    exit 1
  fi
  pass=$((pass+1))
  echo "PASS: $1 rejected"
}
bash "$verify" "$head" >/dev/null
pass=$((pass+1))
reject_wrong="0000000000000000000000000000000000000000"
original="$head"
head="$reject_wrong"
reject 'wrong commit'
head="$original"
printf 'changed\n' > tracked.txt
reject 'unstaged file modification'
git reset -q --hard HEAD
printf 'staged\n' > tracked.txt
git add tracked.txt
reject 'staged file modification'
git reset -q --hard HEAD
printf 'untracked\n' > extra.txt
reject 'untracked file'
rm extra.txt
printf 'ignored\n' > temp.cache
reject 'ignored file'
rm temp.cache
chmod +x tracked.txt
reject 'physical executable bit change'
chmod -x tracked.txt
git update-index --skip-worktree tracked.txt
printf 'hidden changed bytes\n' > tracked.txt
reject 'skip-worktree hidden content mutation'
git update-index --no-skip-worktree tracked.txt
git reset -q --hard HEAD
# Invalid pins must fail before any source attribution.
original="$head"
head="not-a-commit"
reject 'malformed expected commit'
head="$original"
# Untracked symlinks must never silently join the trusted checkout.
ln -s tracked.txt untracked-alias
reject 'untracked symlink'
rm untracked-alias
rm tracked.txt
reject 'missing tracked file'
git reset -q --hard HEAD
# Prove that clean filters cannot conceal altered physical source bytes.
printf 'tracked.txt filter=normalize\n' > .gitattributes
git config filter.normalize.clean "sed 's/ *$//'"
git add .gitattributes
git commit -qm 'normalize-filter trap'
head="$(git rev-parse HEAD)"
printf 'tracked   \n' > tracked.txt
reject 'clean-filter physical-byte laundering'
git reset -q --hard HEAD
# A tracked executable must remain physically executable.
chmod +x tracked.txt
git add tracked.txt
git commit -qm 'tracked executable fixture'
head="$(git rev-parse HEAD)"
chmod -x tracked.txt
reject 'missing tracked executable bit'
git reset -q --hard HEAD
ln -s tracked.txt alias.txt
git add alias.txt
git commit -qm 'tracked symlink negative fixture'
head="$(git rev-parse HEAD)"
reject 'tracked symlink'
# A clean text normalization can conceal modified working bytes from git diff
# and from git hash-object --path. The physical-byte gate must reject it.
git reset -q --hard HEAD^
printf '*.txt text\n' > .gitattributes
git add .gitattributes
git commit -qm 'CRLF normalization adversarial fixture'
head="$(git rev-parse HEAD)"
printf 'tracked\r\n' > tracked.txt
reject 'CRLF text-normalization laundering'
git reset -q --hard HEAD
# Directory symlinks may escape the source checkout even if the leaf is regular.
mkdir -p nested
printf 'nested source\n' > nested/child.txt
git add nested/child.txt
git commit -qm 'nested tracked-file fixture'
head="$(git rev-parse HEAD)"
outside="$(mktemp -d)"
cp nested/child.txt "$outside/child.txt"
rm -rf nested
ln -s "$outside" nested
reject 'tracked directory symlink escape'
rm nested
rm -rf "$outside"
git reset -q --hard HEAD
# Physical hardlinks can hide an external writable alias without changing Git status.
outside="$(mktemp -d)"
ln tracked.txt "$outside/alias.txt"
reject 'tracked physical hardlink escape'
rm -rf "$outside"
# An ls-files failure must be rejected, not treated as an empty successful list.
shim="$(mktemp -d)"
cat > "$shim/git" <<'GIT_SHIM'
#!/usr/bin/env bash
if [[ "$1" == ls-files && " $* " == *" $FAIL_LS_FILES "* ]]; then
  exit 37
fi
exec "$REAL_GIT" "$@"
GIT_SHIM
chmod +x "$shim/git"
real_git="$(command -v git)"
for selector in --stage --ignored --exclude-standard; do
  # --exclude-standard appears in both untracked and ignored; either failure
  # must terminate verification.
  output="$(env REAL_GIT="$real_git" FAIL_LS_FILES="$selector" PATH="$shim:$PATH" \
    bash "$verify" "$head" 2>&1)" && {
    echo "FAIL: Git ls-files failure ($selector) was admitted" >&2
    exit 1
  }
  [[ "$output" == *'cannot enumerate '* ]] || {
    echo "FAIL: missing explicit enumeration error ($selector)" >&2
    exit 1
  }
  pass=$((pass+1))
done
rm -rf "$shim"
# A parent process must not select a forged index, worktree or object store.
for override in GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE \
  GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_CONFIG_COUNT \
  GIT_CONFIG_PARAMETERS; do
  output="$(env "$override=forged" bash "$verify" "$head" 2>&1)" && {
    echo "FAIL: $override override was admitted" >&2
    exit 1
  }
  [[ "$output" == *"Git environment override forbidden: $override"* ]] || {
    echo "FAIL: wrong rejection for $override: $output" >&2
    exit 1
  }
  pass=$((pass+1))
done
# Git tracks an executable bit only: exact bytes and index state do not
# authorize world/group writable or privileged physical checkout modes.
for permission in o+w g+w u+s g+s +t; do
  chmod "$permission" tracked.txt
  reject "unsafe tracked file permissions: $permission"
  case "$permission" in
    o+w) chmod o-w tracked.txt ;;
    g+w) chmod g-w tracked.txt ;;
    u+s) chmod u-s tracked.txt ;;
    g+s) chmod g-s tracked.txt ;;
    +t) chmod -t tracked.txt ;;
  esac
done
# Directory mode is absent from Git tree objects: a writable tracked-path
# ancestor is a checkout replacement boundary, even if tracked leaves match.
for permission in o+w g+w; do
  chmod "$permission" nested
  reject "writable tracked source ancestor: $permission"
  case "$permission" in
    o+w) chmod o-w nested ;;
    g+w) chmod g-w nested ;;
  esac
done
# A file rewritten immediately AFTER its first successful physical hash must
# still be denied. A one-shot Git wrapper reproduces the mid-audit race.
shim="$(mktemp -d)"
cat > "$shim/git" <<'GIT_SHIM'
#!/usr/bin/env bash
if [[ "$1" == hash-object && " $* " == *" tracked.txt "* && ! -e "$TAMPER_ONCE" ]]; then
  touch "$TAMPER_ONCE"
  "$REAL_GIT" "$@"
  printf 'mutated after initial hash\n' > tracked.txt
  exit 0
fi
exec "$REAL_GIT" "$@"
GIT_SHIM
chmod +x "$shim/git"
real_git="$(command -v git)"
output="$(env REAL_GIT="$real_git" TAMPER_ONCE="$shim/once" PATH="$shim:$PATH" \
  bash "$verify" "$head" 2>&1)" && {
  echo 'FAIL: mid-audit source mutation was admitted' >&2
  exit 1
}
[[ "$output" == *'physical source changed during verification'* ]] || {
  echo "FAIL: unexpected mid-audit mutation diagnostic: $output" >&2
  exit 1
}
pass=$((pass+1))
rm -rf "$shim"
git reset -q --hard HEAD
printf 'PASS: %s source-integrity self-checks (1 positive, %s negative)\n' "$pass" "$((pass-1))"
