"""Deliberate lock policy violations must fail (stdlib-only, no network)."""
import copy
import unittest
from check_lock_policy import validate


class LockPolicyTest(unittest.TestCase):
    def setUp(self):
        self.manifest = {
            "package": {"name": "orescranelift", "version": "0.1.0"},
            "dependencies": {"cranelift-codegen": "=0.137.0", "libc": "=0.2.191"},
        }
        self.lock = {
            "version": 4,
            "package": [
                {"name": "orescranelift", "version": "0.1.0",
                 "dependencies": ["cranelift-codegen", "libc"]},
                {"name": "cranelift-codegen", "version": "0.137.0",
                 "source": "registry+https://github.com/rust-lang/crates.io-index",
                 "checksum": "a" * 64},
                {"name": "libc", "version": "0.2.191",
                 "source": "registry+https://github.com/rust-lang/crates.io-index",
                 "checksum": "b" * 64},
            ],
        }

    def test_valid_policy(self):
        validate(self.lock, self.manifest)

    def test_rejects_git_dependency(self):
        invalid = copy.deepcopy(self.lock)
        invalid["package"][1]["source"] = "git+https://example.com/repo"
        with self.assertRaisesRegex(ValueError, "non-crates.io"):
            validate(invalid, self.manifest)

    def test_rejects_missing_checksum(self):
        invalid = copy.deepcopy(self.lock)
        del invalid["package"][2]["checksum"]
        with self.assertRaisesRegex(ValueError, "checksum"):
            validate(invalid, self.manifest)

    def test_rejects_mismatched_direct_pin(self):
        invalid = copy.deepcopy(self.manifest)
        invalid["dependencies"]["libc"] = "=0.2.190"
        with self.assertRaisesRegex(ValueError, "lock mismatch"):
            validate(self.lock, invalid)

    def test_rejects_unpinned_direct_dependency(self):
        invalid = copy.deepcopy(self.manifest)
        invalid["dependencies"]["libc"] = "^0.2"
        with self.assertRaisesRegex(ValueError, "not exactly pinned"):
            validate(self.lock, invalid)

    def test_rejects_unapproved_path_package(self):
        invalid = copy.deepcopy(self.lock)
        invalid["package"].append({"name": "hidden-local", "version": "0.1.0"})
        with self.assertRaisesRegex(ValueError, "unapproved local"):
            validate(invalid, self.manifest)

    def test_rejects_duplicate_package(self):
        invalid = copy.deepcopy(self.lock)
        invalid["package"].append(copy.deepcopy(invalid["package"][1]))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            validate(invalid, self.manifest)

    def test_rejects_cranelift_drift(self):
        invalid = copy.deepcopy(self.lock)
        invalid["package"][1]["version"] = "0.136.0"
        with self.assertRaisesRegex(ValueError, "Cranelift version drift"):
            validate(invalid, self.manifest)

    def test_rejects_root_dependency_mismatch(self):
        invalid = copy.deepcopy(self.lock)
        invalid["package"][0]["dependencies"] = ["libc"]
        with self.assertRaisesRegex(ValueError, "differ"):
            validate(invalid, self.manifest)


if __name__ == "__main__":
    unittest.main()
