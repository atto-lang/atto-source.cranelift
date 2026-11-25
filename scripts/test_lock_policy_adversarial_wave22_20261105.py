"""Adversarial exact dependency-lock controls, source-local and host-only."""
import copy
import unittest
from check_lock_policy import REGISTRY, validate


def fixture():
    manifest = {
        "package": {"name": "orescranelift", "version": "0.1.0"},
        "dependencies": {"cranelift-codegen": "=0.137.0", "libc": "=0.2.191"},
    }
    lock = {
        "version": 4,
        "package": [
            {"name": "orescranelift", "version": "0.1.0",
             "dependencies": ["cranelift-codegen", "libc"]},
            {"name": "cranelift-codegen", "version": "0.137.0",
             "source": REGISTRY, "checksum": "a" * 64},
            {"name": "libc", "version": "0.2.191",
             "source": REGISTRY, "checksum": "b" * 64},
        ],
    }
    return lock, manifest


class LockPolicyWave22Adversarial(unittest.TestCase):
    """Pure lock validation tests; no Rust guest/ABI claims."""

    def test_canonical_exact_manifest_control(self):
        lock, manifest = fixture()
        validate(lock, manifest)

    def test_uppercase_sha256_checksum_never_counts_as_registry_pin(self):
        lock, manifest = fixture()
        lock["package"][1]["checksum"] = "A" * 64
        with self.assertRaisesRegex(ValueError, "checksum"):
            validate(lock, manifest)

    def test_registry_source_with_extra_suffix_is_not_authoritative(self):
        lock, manifest = fixture()
        lock["package"][2]["source"] = REGISTRY + "?substituted=true"
        with self.assertRaisesRegex(ValueError, "non-crates.io"):
            validate(lock, manifest)

    def test_root_local_package_cannot_acquire_checksum_or_source(self):
        for key, value in (("checksum", "c" * 64), ("source", REGISTRY)):
            with self.subTest(key=key):
                lock, manifest = fixture()
                lock["package"][0][key] = value
                with self.assertRaises(ValueError):
                    validate(lock, manifest)

    def test_second_cranelift_release_is_not_silently_admitted(self):
        lock, manifest = fixture()
        other = copy.deepcopy(lock["package"][1])
        other["version"] = "0.136.0"
        lock["package"].append(other)
        with self.assertRaisesRegex(ValueError, "Cranelift version drift"):
            validate(lock, manifest)

    def test_root_dependency_list_cannot_omit_required_direct_pin(self):
        lock, manifest = fixture()
        lock["package"][0]["dependencies"] = ["cranelift-codegen"]
        with self.assertRaisesRegex(ValueError, "differ"):
            validate(lock, manifest)


if __name__ == "__main__":
    unittest.main()
