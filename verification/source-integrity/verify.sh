#!/usr/bin/env bash
# Check physical checkout against a pinned commit before any build step.
set -euo pipefail
fail() { printf 'SOURCE_INTEGRITY_FAIL: %s\n' "$*" >&2; exit 1; }
expected="${1:-}"
[[ "$expected" =~ ^[0-9a-f]{40}$ ]] || fail 'expected SHA must be 40 lowercase hex characters'
# Do not let caller-supplied Git plumbing redirect this trust check to a
# substitute index, worktree, object store or injected configuration.
for git_override in GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE \
  GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_CONFIG_COUNT \
  GIT_CONFIG_PARAMETERS; do
  [[ -z "${!git_override-}" ]] || fail "Git environment override forbidden: $git_override"
done
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || fail 'not a Git worktree'
root="$(git rev-parse --show-toplevel)"
[[ "$(pwd -P)" == "$(cd "$root" && pwd -P)" ]] || fail 'must run at repository root'
[[ "$(git rev-parse HEAD)" == "$expected" ]] || fail 'commit mismatch'
expected_tree="$(git rev-parse "${expected}^{tree}")"
[[ "$(git write-tree)" == "$expected_tree" ]] || fail 'staged tree differs from pinned commit'
git -c core.filemode=true diff --quiet --exit-code -- || fail 'unstaged modifications'
git diff --cached --quiet --exit-code HEAD -- || fail 'staged modifications'
# Bash process substitution hides producer exit codes. Materialize every Git
# enumeration outside the checkout and check its status before trusting output.
# A failed ls-files must never look like an empty clean index.
source_listing="$(mktemp -d)" || fail 'cannot create source listing directory'
trap 'rm -rf -- "$source_listing"' EXIT
git ls-files --others --exclude-standard -z > "$source_listing/untracked" || fail 'cannot enumerate untracked files'
if IFS= read -r -d '' stray < "$source_listing/untracked"; then
  fail "untracked file: $stray"
fi
git ls-files --others --ignored --exclude-standard -z > "$source_listing/ignored" || fail 'cannot enumerate ignored files'
if IFS= read -r -d '' stray < "$source_listing/ignored"; then
  fail "ignored file: $stray"
fi
git ls-files --stage -z > "$source_listing/staged" || fail 'cannot enumerate tracked files'
[[ -s "$source_listing/staged" ]] || fail 'tracked index is empty'
while IFS= read -r -d '' entry; do
  metadata="${entry%%$'\t'*}"
  path="${entry#*$'\t'}"
  mode="${metadata%% *}"
  rest="${metadata#* }"
  blob="${rest%% *}"
  [[ "$mode" == '100644' || "$mode" == '100755' ]] || fail "forbidden Git entry mode $mode: $path"
  # Reject path resolution through directory symlinks before reading tracked bytes.
  parent="${path%/*}"
  if [[ "$parent" != "$path" ]]; then
    while :; do
      [[ -d "$parent" && ! -L "$parent" ]] || fail "symlinked or missing tracked ancestor: $parent ($path)"
      # A writable ancestor lets another principal replace source files after
      # verification, even when each current leaf is immutable and byte-exact.
      ancestor_mode="$(stat -c %a -- "$parent" 2>/dev/null || stat -f %Lp "$parent" 2>/dev/null)" || fail "cannot inspect source directory permissions: $parent"
      [[ "$ancestor_mode" =~ ^[0-7]{3,4}$ ]] || fail "invalid source directory permissions: $parent"
      (( (8#$ancestor_mode & 8#0022) == 0 )) || fail "writable tracked source directory: $parent"
      [[ "$parent" == */* ]] || break
      parent="${parent%/*}"
    done
  fi
  [[ -f "$path" && ! -L "$path" ]] || fail "missing or symlinked tracked file: $path"
  # Git only records executability; unsafe writable or privileged mode bits
  # can survive an otherwise clean index and exact physical byte hash.
  physical_mode="$(stat -c %a -- "$path" 2>/dev/null || stat -f %Lp "$path" 2>/dev/null)" || fail "cannot inspect tracked file permissions: $path"
  [[ "$physical_mode" =~ ^[0-7]{3,4}$ ]] || fail "invalid physical permissions: $path"
  (( (8#$physical_mode & 8#7022) == 0 )) || fail "unsafe tracked file permissions: $path"
  # Git does not record hardlink identity; an external alias can mutate a verified file.
  link_count="$(stat -c %h -- "$path" 2>/dev/null || stat -f %l "$path" 2>/dev/null)" || fail "cannot inspect tracked file link count: $path"
  [[ "$link_count" == "1" ]] || fail "tracked file has an unexpected hardlink: $path"
  # Compare the physical bytes, not a clean-filter-normalized Git view.
  # git hash-object --path silently launders modifications through .gitattributes
  # and configured filter.*.clean commands (git diff can hide them too).
  actual_blob="$(git hash-object --no-filters -- "$path")" || fail "cannot hash tracked file: $path"
  [[ "$actual_blob" == "$blob" ]] || fail "physical file contents differ from pinned blob: $path"
  if [[ "$mode" == '100755' ]]; then
    [[ -x "$path" ]] || fail "executable bit missing: $path"
  else
    [[ ! -x "$path" ]] || fail "unexpected executable bit: $path"
  fi
done < "$source_listing/staged"
printf 'SOURCE_INTEGRITY_OK commit=%s tree=%s\n' "$expected" "$expected_tree"
