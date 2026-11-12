from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest import mock
import zipfile

from scripts import fork_release as release


class ForkReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.name = "herdr-macos-aarch64"
        self.commit = "a" * 40
        self.run_url = "https://github.com/XinqiBao/herdr/actions/runs/123"
        binary = self.directory / self.name
        binary.write_bytes(b"verified binary")
        self.fields = {
            "commit": self.commit,
            "repository": release.REPOSITORY,
            "run_url": self.run_url,
            "run_attempt": "2",
            "build_channel": "fork",
            "build_id": f"xbao.{self.commit[:7]}",
            "target": release.ARTIFACTS[self.name],
            "libghostty_vt_optimize": "ReleaseFast",
            "libghostty_vt_simd": "true",
            "sha256": f"{release.digest(binary)}  {self.name}",
        }

    def write_info(self):
        (self.directory / "BUILD_INFO.txt").write_bytes(
            ("\ufeff" + "\r\n".join(f"{k}={v}" for k, v in self.fields.items())).encode("utf-8")
        )

    def validate(self):
        release.validate_artifact(self.directory, self.name, self.commit, self.run_url, "2")

    def test_metadata_fixture_preserves_utf8_bom_and_crlf_without_locale_encoding(self):
        original_open = io.open

        def require_explicit_encoding(file, mode="r", buffering=-1, encoding=None, *args, **kwargs):
            if "b" not in mode:
                self.assertNotIn(encoding, (None, "locale"), "fixture depends on locale encoding")
            return original_open(file, mode, buffering, encoding, *args, **kwargs)

        with mock.patch("io.open", side_effect=require_explicit_encoding):
            self.write_info()
        data = (self.directory / "BUILD_INFO.txt").read_bytes()
        self.assertTrue(data.startswith(b"\xef\xbb\xbf"))
        self.assertEqual(data.count(b"\r\n"), len(self.fields) - 1)
        self.assertNotIn(b"\n", data.replace(b"\r\n", b""))
        self.assertNotIn(b"\r", data.replace(b"\r\n", b""))
        self.assertEqual(data.decode("utf-8-sig").splitlines(),
                         [f"{k}={v}" for k, v in self.fields.items()])
        self.validate()

    def test_accepts_windows_style_metadata_and_rejects_other_sources_or_settings(self):
        self.write_info()
        self.validate()
        for key in self.fields:
            with self.subTest(key=key):
                original = self.fields[key]
                self.fields[key] = "wrong"
                self.write_info()
                with self.assertRaisesRegex(ValueError, "mismatch"):
                    self.validate()
                self.fields[key] = original

    def test_rejects_modified_binary_duplicate_fields_and_unexpected_files(self):
        self.write_info()
        binary = self.directory / self.name
        binary.write_bytes(b"changed after build")
        with self.assertRaisesRegex(ValueError, "mismatch"):
            self.validate()
        binary.write_bytes(b"verified binary")
        with (self.directory / "BUILD_INFO.txt").open("a", encoding="utf-8", newline="") as info:
            info.write(f"\ncommit={self.commit}\n")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.validate()
        self.write_info()
        (self.directory / "extra").write_bytes(b"unexpected")
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.validate()

    def test_rejects_missing_binary_and_symlinks(self):
        self.write_info()
        binary = self.directory / self.name
        binary.unlink()
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.validate()
        target = self.directory.parent / (self.directory.name + "-target")
        target.write_bytes(b"verified binary")
        self.addCleanup(target.unlink)
        try:
            binary.symlink_to(target)
        except OSError:
            self.skipTest("symlinks unavailable")
        with self.assertRaisesRegex(ValueError, "regular"):
            self.validate()

    def test_uploaded_assets_require_exact_names_digests_and_completion(self):
        expected = {name: "a" * 64 for name in release.ARTIFACTS}
        assets = [{"name": name, "state": "uploaded", "digest": f"sha256:{value}"}
                  for name, value in expected.items()]
        release.validate_uploaded({"assets": assets}, expected)
        for invalid in [assets[:-1], assets + [assets[0]],
                        assets[:-1] + [dict(assets[-1], state="new")],
                        assets[:-1] + [dict(assets[-1], digest="sha256:wrong")]]:
            with self.assertRaises(ValueError):
                release.validate_uploaded({"assets": invalid}, expected)
        with self.assertRaises(ValueError):
            release.validate_uploaded(None, expected)

    def test_canonical_base_accepts_annotated_and_lightweight_tag_peels(self):
        base = {"repository": "herdrdev/herdr", "tag": "v0.9.5", "commit": self.commit}
        manifest = self.directory / "base.json"
        manifest.write_text(json.dumps(base))
        ref = "refs/tags/v0.9.5"
        for output in [f"{'b' * 40}\t{ref}\n{self.commit}\t{ref}^{{}}",
                       f"{self.commit}\t{ref}"]:
            with self.subTest(output=output), mock.patch.object(
                release, "command", side_effect=[output, ""]
            ) as command:
                self.assertEqual(release.verify_base(manifest), base)
                self.assertEqual(command.call_args_list, [
                    mock.call("git", "ls-remote", "https://github.com/herdrdev/herdr.git", ref, ref + "^{}"),
                    mock.call("git", "merge-base", "--is-ancestor", self.commit, "HEAD"),
                ])

    def test_canonical_base_rejects_missing_wrong_sha_and_non_ancestor(self):
        manifest = self.directory / "base.json"
        manifest.write_text(json.dumps({"repository": "herdrdev/herdr", "tag": "v0.9.5",
                                        "commit": self.commit}))
        for output in ["", f"{'b' * 40}\trefs/tags/v0.9.5",
                       f"{self.commit}\trefs/tags/v0.9.5\n{'b' * 40}\trefs/tags/v0.9.5^{{}}"]:
            with self.subTest(output=output), mock.patch.object(release, "command", return_value=output) as command:
                with self.assertRaisesRegex(ValueError, "tag mismatch"):
                    release.verify_base(manifest)
                self.assertEqual(command.call_count, 1)
        with mock.patch.object(release, "command", side_effect=[
            f"{self.commit}\trefs/tags/v0.9.5", subprocess.CalledProcessError(1, "merge-base")
        ]):
            with self.assertRaises(subprocess.CalledProcessError):
                release.verify_base(manifest)

    def test_base_rejects_noncanonical_repository_and_untrusted_tag_syntax(self):
        manifest = self.directory / "base.json"
        for repository, tag in [("XinqiBao/herdr", "v0.9.5"), ("herdrdev/herdr", "v0.9.5; command")]:
            manifest.write_text(json.dumps({"repository": repository, "tag": tag, "commit": self.commit}))
            with mock.patch.object(release, "command") as command:
                with self.assertRaises(ValueError):
                    release.verify_base(manifest)
                command.assert_not_called()

    def test_windows_archive_requires_complete_pinned_bundle(self):
        metadata = {
            "package": {"id": "ConPTY", "version": "1", "sha256": "a" * 64},
            "bundles": {"x86_64": {"files": [{
                "destination": "conpty/conpty.dll", "sha256": hashlib.sha256(b"dll").hexdigest(),
            }]}},
            "notices": [],
        }
        payload = {
            "herdr.exe": b"executable",
            "conpty/conpty.dll": b"dll",
            release.conpty.MARKER_PATH.as_posix(): release.conpty.marker_data(metadata, "x86_64"),
        }
        archive = self.directory / "windows.zip"

        def write_zip(files):
            with zipfile.ZipFile(archive, "w") as output:
                for name, data in files.items():
                    output.writestr(name, data)

        with mock.patch.object(release.conpty, "validate_static_msvc_runtime") as validate_pe:
            write_zip(payload)
            release.validate_windows(archive, metadata)
            validate_pe.assert_called_once_with(b"executable", "herdr.exe")
            for invalid in [dict(payload, extra=b"unexpected"),
                            {k: v for k, v in payload.items() if k != "conpty/conpty.dll"},
                            dict(payload, **{"conpty/conpty.dll": b"wrong"})]:
                write_zip(invalid)
                with self.assertRaises(ValueError):
                    release.validate_windows(archive, metadata)


if __name__ == "__main__":
    unittest.main()
