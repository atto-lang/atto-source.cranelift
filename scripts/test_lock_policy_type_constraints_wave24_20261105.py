"""Fail-closed lock validator: root, registry, checksum and semver identity controls."""
import copy
import unittest
from check_lock_policy import validate, REGISTRY, CRANELIFT_VERSION


class LockPolicyTypeConstraintsWave24(unittest.TestCase):
    def setUp(self):
        self.manifest = {
            "package": {"name": "orescranelift", "version": "0.1.0"},
            "dependencies": {"cranelift-codegen": "=" + CRANELIFT_VERSION, "libc": "=0.2.191"},
        }
        self.lock = {
            "version": 4,
            "package": [
                {"name": "orescranelift", "version": "0.1.0",
                 "dependencies": ["cranelift-codegen", "libc"]},
                {"name": "cranelift-codegen", "version": CRANELIFT_VERSION,
                 "source": REGISTRY, "checksum": "a" * 64},
                {"name": "libc", "version": "0.2.191",
                 "source": REGISTRY, "checksum": "b" * 64},
            ],
        }

    def reject(self, lock=None, manifest=None, message=None):
        with self.assertRaisesRegex(ValueError, message or "."):
            validate(lock if lock is not None else self.lock,
                     manifest if manifest is not None else self.manifest)

    def test_valid_exact_registry_pins_and_transitive_package(self):
        lock = copy.deepcopy(self.lock)
        lock["package"].append({"name": "cranelift-jit", "version": CRANELIFT_VERSION,
                                "source": REGISTRY, "checksum": "c" * 64})
        validate(lock, self.manifest)

    def test_local_root_cannot_claim_registry_checksum(self):
        lock = copy.deepcopy(self.lock)
        lock["package"][0]["checksum"] = "d" * 64
        self.reject(lock=lock, message="unapproved local")

    def test_local_root_cannot_be_omitted(self):
        lock = copy.deepcopy(self.lock)
        lock["package"].pop(0)
        self.reject(lock=lock, message="exactly one root")

    def test_registry_requires_exact_source_url(self):
        lock = copy.deepcopy(self.lock)
        lock["package"][1]["source"] = REGISTRY + "/"
        self.reject(lock=lock, message="non-crates.io")

    def test_checksum_must_be_lowercase_hex_sha256(self):
        for checksum in ("A" * 64, "f" * 63, "g" * 64, 1234):
            with self.subTest(checksum=checksum):
                lock = copy.deepcopy(self.lock)
                lock["package"][2]["checksum"] = checksum
                self.reject(lock=lock, message="checksum")

    def test_transitive_cranelift_version_cannot_drift(self):
        lock = copy.deepcopy(self.lock)
        lock["package"].append({"name": "cranelift-jit", "version": "0.136.0",
                                "source": REGISTRY, "checksum": "c" * 64})
        self.reject(lock=lock, message="Cranelift version drift")

    def test_direct_dependency_exact_pin_must_equal_lock(self):
        manifest = copy.deepcopy(self.manifest)
        manifest["dependencies"]["libc"] = "=0.2.190"
        self.reject(manifest=manifest, message="lock mismatch")

    def test_lock_format_is_exact_v4(self):
        for version in (3, 5, "4", None):
            with self.subTest(version=version):
                lock = copy.deepcopy(self.lock)
                lock["version"] = version
                self.reject(lock=lock, message="format must be v4")

    def test_second_local_package_is_forbidden(self):
        lock = copy.deepcopy(self.lock)
        lock["package"].append({"name": "hidden-path", "version": "0.1.0"})
        self.reject(lock=lock, message="unapproved local")


if __name__ == "__main__":
    unittest.main(verbosity=2)
