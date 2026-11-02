#!/usr/bin/env python3
"""Verify a pinned SHA-1 checkout without executing repository worktree filters."""
import hashlib
import os
import re
import stat
import subprocess
import sys
import unicodedata

SHA = re.compile(r"[0-9a-f]{40}\Z")
MAX_FILES = 2000
MAX_BYTES = 64 * 1024 * 1024
MAX_FILE = 8 * 1024 * 1024


def git(*args):
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env["GIT_NO_REPLACE_OBJECTS"] = "1"
    return subprocess.check_output(
        ["git", "-c", "core.fsmonitor=false", "--no-optional-locks", *args],
        env=env, stderr=subprocess.DEVNULL,
    )


def safe_name(raw):
    if not raw or raw.startswith(b"/") or b"\\" in raw:
        return None
    name = raw.decode("utf-8", "strict")
    if unicodedata.normalize("NFC", name) != name:
        return None
    for component in name.split("/"):
        if component in ("", ".", "..") or component.casefold() == ".git":
            return None
        if component.endswith((" ", ".")) or any(
            ord(ch) < 32 or ord(ch) == 127 or ch in '<>:"\\|?*' for ch in component
        ):
            return None
    return name


def objects():
    tree, names, parents = {}, set(), {}
    for row in git("ls-tree", "-r", "-z", "HEAD").split(b"\0"):
        if not row:
            continue
        header, raw = row.split(b"\t", 1)
        mode, kind, blob = header.split()
        name = safe_name(raw)
        if kind != b"blob" or mode not in (b"100644", b"100755") or not SHA.fullmatch(blob.decode()):
            raise ValueError("unsupported Git tree object")
        if name is None or name.casefold() in names:
            raise ValueError("nonportable or colliding tree path")
        parts = name.split("/")
        for i in range(1, len(parts) + 1):
            spelled = tuple(parts[:i])
            folded = tuple(component.casefold() for component in spelled)
            if folded in parents and parents[folded] != spelled:
                raise ValueError("case-colliding parent path")
            parents[folded] = spelled
        names.add(name.casefold())
        tree[raw] = (mode, blob)
    if not tree or len(tree) > MAX_FILES:
        raise ValueError("invalid tracked-file budget")
    return tree


def index():
    entries = {}
    for row in git("ls-files", "--stage", "-z").split(b"\0"):
        if not row:
            continue
        header, raw = row.split(b"\t", 1)
        mode, blob, stage = header.split()
        if stage != b"0" or raw in entries:
            raise ValueError("unmerged or duplicate index record")
        entries[raw] = mode, blob
    if any(row and not row.startswith(b"H ") for row in git("ls-files", "-v", "-z").split(b"\0")):
        raise ValueError("hidden index entry")
    return entries


def raw_blob(root_fd, raw_path, mode):
    opened = []
    parent = root_fd
    try:
        components = raw_path.split(b"/")
        for component in components[:-1]:
            parent = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            opened.append(parent)
        fd = os.open(components[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        opened.append(fd)
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_FILE:
            raise ValueError("unsafe source file type/size")
        if bool(before.st_mode & 0o111) != (mode == b"100755"):
            raise ValueError("source file mode changed")
        buf = bytearray()
        while len(buf) <= MAX_FILE:
            chunk = os.read(fd, min(65536, MAX_FILE + 1 - len(buf)))
            if not chunk:
                break
            buf.extend(chunk)
        after = os.fstat(fd)
        key = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        if len(buf) != before.st_size or len(buf) > MAX_FILE or key(before) != key(after):
            raise ValueError("source changed while being read")
        return bytes(buf)
    finally:
        for fd in reversed(opened):
            os.close(fd)


def verify(expected):
    if not SHA.fullmatch(expected or ""):
        raise ValueError("missing full expected SHA")
    if git("rev-parse", "--show-object-format").strip() != b"sha1":
        raise ValueError("unsupported Git hash algorithm")
    if git("rev-parse", "HEAD").decode().strip() != expected:
        raise ValueError("wrong HEAD revision")
    if git("for-each-ref", "--format=%(refname)", "refs/replace").strip():
        raise ValueError("replace refs present")
    tree = objects()
    if tree != index():
        raise ValueError("index differs from pinned HEAD tree")
    if git("ls-files", "--others", "-z"):
        raise ValueError("untracked source exists")
    total = 0
    root_fd = os.open(b".", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for raw, (mode, oid) in tree.items():
            body = raw_blob(root_fd, raw, mode)
            total += len(body)
            if total > MAX_BYTES:
                raise ValueError("checkout too large")
            actual = hashlib.sha1(b"blob " + str(len(body)).encode() + b"\0" + body).hexdigest().encode()
            if actual != oid:
                raise ValueError("worktree bytes differ from HEAD")
    finally:
        os.close(root_fd)
    if git("rev-parse", "HEAD").decode().strip() != expected or tree != index():
        raise ValueError("checkout changed during verification")
    if git("ls-files", "--others", "-z"):
        raise ValueError("untracked material appeared")


if __name__ == "__main__":
    try:
        verify(os.environ.get("EXPECTED_SHA", ""))
        print("RAW_GIT_TREE_OK")
    except (ValueError, OSError, UnicodeError, subprocess.CalledProcessError) as error:
        print("RAW_GIT_TREE_REJECTED: " + str(error), file=sys.stderr)
        sys.exit(1)
