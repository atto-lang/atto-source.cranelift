#!/usr/bin/env python3
"""Fail closed on unlocked, non-registry or unexpectedly unpinned Rust dependencies.

This checks structural lock integrity, not security advisories or publisher trust.
"""
from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
CRANELIFT_VERSION = "0.137.0"


def validate(lock: dict, manifest: dict) -> None:
    if lock.get("version") != 4:
        raise ValueError("Cargo.lock format must be v4")
    packages = lock.get("package")
    if not isinstance(packages, list) or not packages:
        raise ValueError("Cargo.lock must include package entries")
    local = (manifest["package"]["name"], manifest["package"]["version"])
    seen: set[tuple[str, str]] = set()
    locked: dict[str, set[str]] = {}
    local_packages = 0
    for pkg in packages:
        name, version = pkg["name"], pkg["version"]
        key = (name, version)
        if key in seen:
            raise ValueError(f"duplicate locked package: {name} {version}")
        seen.add(key)
        locked.setdefault(name, set()).add(version)
        if "source" not in pkg:
            if key != local or "checksum" in pkg:
                raise ValueError(f"unapproved local/path package: {name} {version}")
            local_packages += 1
            continue
        if pkg["source"] != REGISTRY:
            raise ValueError(f"non-crates.io package source: {name} {version}")
        checksum = pkg.get("checksum")
        if not isinstance(checksum, str) or not SHA256.fullmatch(checksum):
            raise ValueError(f"missing/invalid SHA-256 checksum: {name} {version}")
        if name.startswith("cranelift-") and version != CRANELIFT_VERSION:
            raise ValueError(f"Cranelift version drift: {name} {version}")
    if local_packages != 1:
        raise ValueError("exactly one root local package is required")

    root = next(p for p in packages if (p["name"], p["version"]) == local)
    declared = manifest.get("dependencies", {})
    if not isinstance(declared, dict) or not declared:
        raise ValueError("direct dependencies are missing")
    if set(root.get("dependencies", [])) != set(declared):
        raise ValueError("root locked dependencies differ from Cargo.toml")
    for name, pin in declared.items():
        if not isinstance(pin, str) or not pin.startswith("=") or len(pin) < 2:
            raise ValueError(f"direct dependency is not exactly pinned: {name}")
        if locked.get(name) != {pin[1:]}:
            raise ValueError(f"direct dependency lock mismatch: {name}")


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    try:
        with (root / "Cargo.lock").open("rb") as f:
            lock = tomllib.load(f)
        with (root / "Cargo.toml").open("rb") as f:
            manifest = tomllib.load(f)
        validate(lock, manifest)
    except (OSError, KeyError, TypeError, ValueError, tomllib.TOMLDecodeError) as exc:
        print(f"FAIL dependency lock policy: {exc}", file=sys.stderr)
        return 1
    print(f"PASS lock policy: {len(lock['package'])} packages; only checksummed crates.io dependencies")
    print("NOTE: lock policy is not a RustSec/security-advisory clearance")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
