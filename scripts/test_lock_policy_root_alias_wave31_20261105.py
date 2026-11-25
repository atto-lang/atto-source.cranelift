"""Exact pin and root identity tests of the source lock-policy checker.

Host structural policy only; no cargo build, compiler guest or advisory clearance.
"""
import copy
import unittest
from check_lock_policy import REGISTRY, CRANELIFT_VERSION, validate


class LockRootIdentityWave31(unittest.TestCase):
    def setUp(self):
        self.manifest = {
            "package": {"name": "atto-cranelift", "version": "0.1.0"},
            "dependencies": {"cranelift-codegen": "=" + CRANELIFT_VERSION},
        }
        self.lock = {"version": 4, "package": [
            {"name": "atto-cranelift", "version": "0.1.0",
             "dependencies": ["cranelift-codegen"]},
            {"name": "cranelift-codegen", "version": CRANELIFT_VERSION,
             "source": REGISTRY, "checksum": "a" * 64},
        ]}

    def test_positive_exact_root_and_registry_pin(self):
        validate(self.lock, self.manifest)

    def test_extra_unapproved_path_package_is_rejected(self):
        lock = copy.deepcopy(self.lock)
        lock["package"].append({"name": "shadow", "version": "0.1.0"})
        with self.assertRaisesRegex(ValueError, "unapproved local/path package"):
            validate(lock, self.manifest)

    def test_duplicate_root_identity_is_rejected(self):
        lock = copy.deepcopy(self.lock)
        lock["package"].append(copy.deepcopy(lock["package"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate locked package"):
            validate(lock, self.manifest)

    def test_foreign_registry_and_unpinned_manifest_are_rejected(self):
        lock = copy.deepcopy(self.lock)
        lock["package"][1]["source"] = "git+https://example.invalid/a"
        with self.assertRaisesRegex(ValueError, "non-crates.io"):
            validate(lock, self.manifest)
        manifest = copy.deepcopy(self.manifest)
        manifest["dependencies"]["cranelift-codegen"] = CRANELIFT_VERSION
        with self.assertRaisesRegex(ValueError, "not exactly pinned"):
            validate(self.lock, manifest)


if __name__ == "__main__":
    unittest.main()
