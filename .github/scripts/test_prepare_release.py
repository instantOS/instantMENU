#!/usr/bin/env python3
"""Unit tests for release policy and primary metadata updates."""

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("prepare-release.py")
SPEC = importlib.util.spec_from_file_location("prepare_release", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleasePolicyTests(unittest.TestCase):
    def commit(self, subject: str, body: str = "") -> release.Commit:
        return ("deadbeef", subject, body)

    def test_explicit_bump_wins(self) -> None:
        self.assertEqual(release.bump_level([], "minor"), "minor")

    def test_automatic_bump_levels(self) -> None:
        self.assertIsNone(release.bump_level([self.commit("docs: explain it")], "auto"))
        self.assertEqual(release.bump_level([self.commit("fix: stop crash")], "auto"), "patch")
        self.assertEqual(release.bump_level([self.commit("feat: add mode")], "auto"), "minor")
        self.assertEqual(release.bump_level([self.commit("feat!: replace CLI")], "auto"), "major")
        self.assertEqual(
            release.bump_level([self.commit("fix: change API", "BREAKING CHANGE: API")], "auto"),
            "major",
        )

    def test_non_conventional_work_is_a_patch(self) -> None:
        self.assertEqual(release.bump_level([self.commit("repair launcher")], "auto"), "patch")

    def test_version_bumps(self) -> None:
        self.assertEqual(release.bump_version("1.2.3", "patch"), "1.2.4")
        self.assertEqual(release.bump_version("1.2.3", "minor"), "1.3.0")
        self.assertEqual(release.bump_version("1.2.3", "major"), "2.0.0")


class ReleaseFilesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.old_root = release.ROOT
        release.ROOT = Path(self.temporary_directory.name)

    def tearDown(self) -> None:
        release.ROOT = self.old_root
        self.temporary_directory.cleanup()

    def write(self, path: str, text: str) -> None:
        target = release.ROOT / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def test_primary_versions_change_but_generated_files_do_not(self) -> None:
        self.write("Cargo.toml", '[package]\nname = "instantmenu"\nversion = "1.2.3"\n')
        self.write('Cargo.lock', '[[package]]\nname = "instantmenu"\nversion = "1.2.3"\n')
        for directory in ("packaging/arch", "packaging/arch-bin"):
            self.write(f"{directory}/PKGBUILD", "pkgver=1.2.3\npkgrel=7\n")
            self.write(f"{directory}/.SRCINFO", "generated 1.2.3\n")
        self.write("instantmenu.1", "generated 1.2.3\n")

        release.update_primary_versions("1.2.4")

        self.assertIn('version = "1.2.4"', release.read("Cargo.toml"))
        self.assertIn('version = "1.2.4"', release.read("Cargo.lock"))
        for directory in ("packaging/arch", "packaging/arch-bin"):
            self.assertEqual(release.read(f"{directory}/PKGBUILD"), "pkgver=1.2.4\npkgrel=1\n")
            self.assertEqual(release.read(f"{directory}/.SRCINFO"), "generated 1.2.3\n")
        self.assertEqual(release.read("instantmenu.1"), "generated 1.2.3\n")

    def test_missing_expected_field_fails_loudly(self) -> None:
        self.write("Cargo.toml", "[package]\nname = \"instantmenu\"\n")
        with self.assertRaises(SystemExit):
            release.update_primary_versions("1.2.4")

    def test_changelog_groups_and_deduplicates_commits(self) -> None:
        self.write("CHANGELOG.md", "# Changelog\n\n## [Unreleased]\n")
        commits = [
            ("1", "fix: stop crash", ""),
            ("2", "feat: add mode", ""),
            ("3", "fix: stop crash", ""),
        ]

        release.update_changelog("1.3.0", "v1.2.3", commits)
        changelog = release.read("CHANGELOG.md")

        self.assertIn("instantMENU/compare/v1.2.3...v1.3.0", changelog)
        self.assertIn("### Added\n\n- add mode", changelog)
        self.assertIn("### Fixed\n\n- stop crash", changelog)
        self.assertEqual(changelog.count("- stop crash"), 1)


if __name__ == "__main__":
    unittest.main()
