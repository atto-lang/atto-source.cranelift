"""Adversarial subprocess tests for the pre-compiler raw checkout verifier."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CHECKER = Path(__file__).resolve().parents[1] / "scripts" / "verify_checkout_raw.py"


class RawCheckoutTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="atto-git-raw-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Atto Audit")
        self.git("config", "user.email", "audit@example.invalid")
        (self.root / "main.rs").write_text("fn main() {}\n")
        self.commit("main.rs")

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.root, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True
        ).stdout.strip()

    def commit(self, *paths):
        self.git("add", "--", *paths)
        self.git("commit", "-qm", "fixture")

    def check(self, head=None):
        env = dict(os.environ, EXPECTED_SHA=head or self.git("rev-parse", "HEAD"))
        return subprocess.run(
            [sys.executable, str(CHECKER)], cwd=self.root, env=env,
            text=True, capture_output=True
        )

    def test_clean_checkout(self):
        self.assertEqual(self.check().returncode, 0)

    def test_wrong_head(self):
        self.assertNotEqual(self.check("a" * 40).returncode, 0)

    def test_mutated_source(self):
        (self.root / "main.rs").write_text("not pinned\n")
        self.assertNotEqual(self.check().returncode, 0)

    def test_untracked_file(self):
        (self.root / "untracked").write_text("secret")
        self.assertNotEqual(self.check().returncode, 0)

    def test_symlink(self):
        (self.root / "alias").symlink_to("main.rs")
        self.commit("alias")
        self.assertNotEqual(self.check().returncode, 0)

    def test_mode_drift(self):
        (self.root / "main.rs").chmod(0o755)
        self.assertNotEqual(self.check().returncode, 0)

    def test_parent_case_collision(self):
        (self.root / "src").mkdir()
        (self.root / "SRC").mkdir()
        (self.root / "src" / "a").write_text("a")
        (self.root / "SRC" / "b").write_text("b")
        self.commit("src/a", "SRC/b")
        self.assertNotEqual(self.check().returncode, 0)

    def test_git_clean_filter_never_called(self):
        (self.root / ".gitattributes").write_text("main.rs filter=malicious\n")
        self.commit(".gitattributes")
        sentinel = self.root / "filter-triggered"
        # The file is already tracked. Git diff could invoke this filter;
        # filter-free raw-byte verification must never run the command.
        self.git("config", "filter.malicious.clean", f'sh -c "touch {sentinel}; cat"')
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(sentinel.exists())


if __name__ == "__main__":
    unittest.main()
