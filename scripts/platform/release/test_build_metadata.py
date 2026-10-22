"""Exercise the real metadata build script across a shared Cargo worktree cache."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class BuildMetadataTest(unittest.TestCase):
    def test_release_commit_refreshes_shared_worktree_cache_and_rejects_mismatch(self):
        repository = Path(__file__).resolve().parents[3]
        with tempfile.TemporaryDirectory(prefix="opencoder-build-metadata-") as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "src").mkdir()
            core = source / "crates/metadata-core"
            (core / "src").mkdir(parents=True)
            (source / "Cargo.toml").write_text(
                '[package]\nname="metadata-fixture"\nversion="0.1.0"\n'
                'edition="2021"\n[dependencies]\n'
                'metadata-core={path="crates/metadata-core"}\n'
                '[workspace]\nmembers=["crates/metadata-core"]\n'
            )
            (source / "src/main.rs").write_text(
                'fn main() { println!("{}", metadata_core::commit()); }\n'
            )
            (core / "Cargo.toml").write_text(
                '[package]\nname="metadata-core"\nversion="0.1.0"\n'
                'edition="2021"\nbuild="build.rs"\n'
            )
            (core / "src/lib.rs").write_text(
                "pub fn commit() -> &'static str { env!(\"OPENCODER_GIT_COMMIT_FULL\") }\n"
            )
            shutil.copyfile(repository / "crates/core/build.rs", core / "build.rs")
            (source / ".gitignore").write_text("Cargo.lock\ntarget/\n")

            def git(*arguments):
                return subprocess.check_output(
                    ["git", "-C", str(source), *arguments], text=True,
                    stderr=subprocess.PIPE,
                ).strip()

            git("init", "--quiet")
            git("config", "user.name", "Build fixture")
            git("config", "user.email", "fixture@example.invalid")
            git("add", ".")
            git("commit", "--quiet", "-m", "First fixture revision")
            first = git("rev-parse", "HEAD")
            previous = root / "previous"
            git("worktree", "add", "--quiet", "--detach", str(previous), first)
            (source / "README.md").write_text("Second fixture revision.\n")
            git("add", "README.md")
            git("commit", "--quiet", "-m", "Second fixture revision")
            second = git("rev-parse", "HEAD")
            environment = {
                **os.environ,
                "CARGO_TARGET_DIR": str(root / "shared-target"),
                "OPENCODER_SPA_SHA256": "0" * 64,
            }

            def build(worktree, expected):
                return subprocess.run(
                    ["cargo", "run", "--quiet", "--offline", "--manifest-path",
                     str(worktree / "Cargo.toml")],
                    cwd=worktree,
                    env={**environment, "OPENCODER_BUILD_COMMIT": expected},
                    capture_output=True, text=True, timeout=180,
                )

            for worktree, expected in [(previous, first), (source, second), (previous, first)]:
                result = build(worktree, expected)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), expected)
            rejected = build(previous, second)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("release build commit differs from checkout", rejected.stderr)


if __name__ == "__main__":
    unittest.main()
