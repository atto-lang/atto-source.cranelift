"""Source-backed fail-closed Cargo.lock admission; no Rust compiler or guest execution."""
from copy import deepcopy
import unittest
from check_lock_policy import validate, REGISTRY


def fixture():
    manifest = {'package': {'name': 'candidate', 'version': '0.1.0'},
                'dependencies': {'cranelift-codegen': '=0.137.0', 'serde': '=1.0.0'}}
    lock = {'version': 4, 'package': [
        {'name': 'candidate', 'version': '0.1.0', 'dependencies': ['cranelift-codegen', 'serde']},
        {'name': 'cranelift-codegen', 'version': '0.137.0', 'source': REGISTRY, 'checksum': 'a' * 64},
        {'name': 'serde', 'version': '1.0.0', 'source': REGISTRY, 'checksum': 'b' * 64},
    ]}
    return lock, manifest


class DirectPinWave28Tests(unittest.TestCase):
    def test_minimal_exact_pinned_registry_fixture_passes(self):
        validate(*fixture())

    def test_wildcard_and_implicit_version_declarations_fail(self):
        for bad in ['0.137.0', '^0.137', '*', '>=0.137.0']:
            lock, manifest = fixture()
            manifest['dependencies']['cranelift-codegen'] = bad
            with self.subTest(bad=bad), self.assertRaisesRegex(ValueError, 'not exactly pinned'):
                validate(lock, manifest)

    def test_registry_and_checksum_changes_are_rejected(self):
        for field, bad, diagnostic in [
            ('source', 'git+https://github.com/other/crate', 'non-crates.io'),
            ('source', '', 'non-crates.io'),
            ('checksum', 'A' * 64, 'invalid SHA-256'),
            ('checksum', 'b' * 63, 'invalid SHA-256'),
        ]:
            lock, manifest = fixture()
            lock['package'][2][field] = bad
            with self.subTest(field=field, bad=bad), self.assertRaisesRegex(ValueError, diagnostic):
                validate(lock, manifest)

    def test_declared_and_locked_versions_must_be_identical(self):
        lock, manifest = fixture()
        lock['package'][2]['version'] = '1.0.1'
        with self.assertRaisesRegex(ValueError, 'direct dependency lock mismatch'):
            validate(lock, manifest)

    def test_cranelift_family_version_is_fenced_even_if_manifest_matches(self):
        lock, manifest = fixture()
        lock['package'][1]['version'] = '0.136.0'
        manifest['dependencies']['cranelift-codegen'] = '=0.136.0'
        with self.assertRaisesRegex(ValueError, 'Cranelift version drift'):
            validate(lock, manifest)

    def test_duplicate_root_and_extra_local_package_are_denied(self):
        lock, manifest = fixture()
        lock['package'].append(deepcopy(lock['package'][0]))
        with self.assertRaisesRegex(ValueError, 'duplicate locked package'):
            validate(lock, manifest)
        lock, manifest = fixture()
        lock['package'].append({'name': 'unreviewed-path', 'version': '1.0.0'})
        with self.assertRaisesRegex(ValueError, 'unapproved local/path package'):
            validate(lock, manifest)

    def test_root_dependency_identity_is_exact(self):
        lock, manifest = fixture()
        lock['package'][0]['dependencies'] = ['cranelift-codegen']
        with self.assertRaisesRegex(ValueError, 'root locked dependencies differ'):
            validate(lock, manifest)


if __name__ == '__main__':
    unittest.main()
