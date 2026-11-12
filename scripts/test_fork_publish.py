"""Exercise real command sequencing against an in-memory Git/gh API backend."""

from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from scripts import fork_publish as publish


OLD = "a" * 40
NEW = "b" * 40


class PublisherEntrypointTests(unittest.TestCase):
    def test_main_accepts_only_exact_canonical_https_origins_before_dispatch(self):
        valid = ["https://github.com/XinqiBao/herdr", "https://github.com/XinqiBao/herdr.git"]
        invalid = [
            "https://example.com/XinqiBao/herdr.git",
            "https://github.com/herdrdev/herdr.git",
            "https://github.com/XinqiBao/other",
            "https://github.com/XinqiBao/herdr.git.evil",
            "https://github.com/XinqiBao/herdr/",
            "https://github.com/XinqiBao/herdr.git/extra",
            "https://github.com/XinqiBao/herdr?ref=main",
            "https://github.com/XinqiBao/herdr.git#fragment",
            "https://github.com@evil.example/XinqiBao/herdr",
            "http://github.com/XinqiBao/herdr",
            "git@github.com:XinqiBao/herdr.git",
        ]
        for mode in ["snapshot", "publish"]:
            for origin in valid + invalid:
                with self.subTest(mode=mode, origin=origin), mock.patch.dict(os.environ, {
                    "GITHUB_REPOSITORY": publish.REPO, "GITHUB_SHA": NEW,
                    "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2",
                    "BACKUP_ARTIFACT_ID": "456",
                }, clear=True), mock.patch.object(publish, "Native") as native, \
                        mock.patch.object(publish, "snapshot") as snapshot, \
                        mock.patch.object(publish, "publish") as replace:
                    backend = native.return_value
                    backend.run.side_effect = [NEW + "\n", origin + "\n"]
                    if origin in valid:
                        publish.main(mode)
                        if mode == "snapshot":
                            snapshot.assert_called_once_with(backend, NEW)
                            replace.assert_not_called()
                        else:
                            replace.assert_called_once_with(backend, NEW, "123.2", "456")
                            snapshot.assert_not_called()
                    else:
                        with self.assertRaisesRegex(ValueError, "unexpected origin"):
                            publish.main(mode)
                        snapshot.assert_not_called()
                        replace.assert_not_called()
                    self.assertEqual(backend.run.call_args_list, [
                        mock.call("git", "rev-parse", "HEAD"),
                        mock.call("git", "remote", "get-url", "origin"),
                    ])


class FakeNative(publish.Native):
    def __init__(self, existing=True, unrelated_latest=False):
        self.calls = []
        self.failure = None
        self.failure_after = False
        self.external = None
        self.bad_verification = False
        self.upload_count = 0
        self.hide_after_create = 0
        self.hidden_reads = 0
        self.next_id = 100
        self.objects = {"c" * 40: OLD}
        self.raw_tag = "c" * 40 if existing else None
        self.latest = 9 if unrelated_latest else (7 if existing else None)
        self.files = {name: b"previous " + name.encode() for name in publish.NAMES}
        self.files["PUBLISH_INFO.json"] = json.dumps({"source_commit": OLD}).encode()
        self.release = None
        if existing:
            self.release = {
                "id": 7, "tag_name": publish.TAG, "target_commitish": OLD,
                "name": "Previous title", "body": "Previous notes", "draft": False,
                "prerelease": False, "immutable": False,
                "assets": [self.asset(n, data, "previous label") for n, data in self.files.items()],
            }

    def asset(self, name, data, label=None):
        self.next_id += 1
        return {"id": self.next_id, "name": name, "size": len(data), "label": label,
                "digest": "sha256:" + hashlib.sha256(data).hexdigest(), "state": "uploaded"}

    def fail(self, stage, after=False, external=None):
        self.failure, self.failure_after, self.external = stage, after, external

    def boundary(self, stage, after=False):
        if self.failure == stage and self.failure_after == after:
            self.failure = None
            if self.external:
                self.external(self)
            raise subprocess.CalledProcessError(1, stage, stderr="injected lost response or failure")

    def run(self, *args, data=None):
        self.calls.append(args)
        if args[0] == "git":
            if args[1:3] == ("ls-remote", "origin"):
                if args[3] == "refs/heads/xbao/main":
                    return NEW + "\trefs/heads/xbao/main\n"
                if self.raw_tag is None:
                    return ""
                return f"{self.raw_tag}\t{publish.REF}\n{self.objects[self.raw_tag]}\t{publish.REF}^{{}}\n"
            if args[1] == "fetch":
                return ""
            if args[1] == "rev-parse":
                return self.raw_tag + "\n"
            if args[1] == "cat-file":
                return "tag\n" if args[2] == "-t" else "previous annotated tag\n"
            if args[1] == "mktag":
                value = hashlib.sha1(data.encode()).hexdigest()
                self.objects[value] = data.splitlines()[0].split()[1]
                return value + "\n"
            if "push" in args:
                expected = next(a.split(":", 1)[1] for a in args if a.startswith("--force-with-lease="))
                replacement = args[-1].split(":", 1)[0] or None
                stage = "rollback" if replacement != self.raw_tag and replacement in {None, "c" * 40} else "push"
                self.boundary(stage)
                if (self.raw_tag or "") != expected:
                    raise subprocess.CalledProcessError(1, args, stderr="stale lease")
                self.raw_tag = replacement
                self.boundary(stage, True)
                return ""
        if args[:3] == ("gh", "api", "--paginate"):
            result = copy.deepcopy(self.release)
            if self.hidden_reads:
                self.hidden_reads -= 1
                result = None
            if self.bad_verification and self.upload_count == len(publish.NAMES):
                self.bad_verification = False
                result["assets"][0]["state"] = "new"
            return json.dumps([[result] if result else []])
        if args[:2] == ("gh", "api"):
            path = args[2].removeprefix(f"repos/{publish.REPO}/")
            method = args[args.index("--method") + 1]
            payload = json.loads(data) if data else None
            if path == "releases/latest":
                self.boundary("read")
                if self.latest is None:
                    raise subprocess.CalledProcessError(1, args, stderr="gh: Not Found (HTTP 404)")
                return json.dumps({"id": self.latest})
            if method == "POST":
                self.boundary("create")
                self.release = {k: v for k, v in payload.items() if k != "make_latest"}
                self.release.update(id=8, immutable=False, assets=[])
                self.hidden_reads = self.hide_after_create
                self.boundary("create", True)
                return json.dumps(self.release)
            if method == "PATCH":
                target = int(path.split("/")[-1])
                stage = "edit" if payload.get("body", "").startswith("<!-- xbao-publisher:") else "restore-metadata"
                self.boundary(stage)
                if self.release and target == self.release["id"]:
                    self.release.update({k: v for k, v in payload.items() if k != "make_latest"})
                if payload.get("make_latest") == "true":
                    self.latest = target
                elif payload.get("make_latest") == "false" and self.latest == target:
                    self.latest = None
                self.boundary(stage, True)
                return json.dumps(self.release)
            if method == "DELETE":
                self.boundary("delete")
                if self.latest == self.release["id"]:
                    self.latest = None
                self.release = None
                self.boundary("delete", True)
                return ""
        if args[:3] == ("gh", "release", "download"):
            directory = Path(args[args.index("--dir") + 1])
            for name, contents in self.files.items():
                (directory / name).write_bytes(contents)
            return ""
        if args[:3] == ("gh", "release", "upload"):
            argument = args[4]
            filename, _, label = argument.partition("#")
            path = Path(filename)
            stage = "restore-upload" if path.parent.name == "assets" else "upload"
            self.boundary(stage)
            self.release["assets"] = [a for a in self.release["assets"] if a["name"] != path.name]
            self.boundary(stage + "-deleted")
            contents = path.read_bytes()
            self.files[path.name] = contents
            self.release["assets"].append(self.asset(path.name, contents, label or None))
            if stage == "upload":
                self.upload_count += 1
            self.boundary(stage, True)
            return ""
        raise AssertionError(f"unexpected command: {args}")


class ForkPublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.output, self.backup, self.notes = root / "new", root / "backup", root / "notes.md"
        self.output.mkdir()
        for name in publish.NAMES:
            (self.output / name).write_bytes(b"new " + name.encode())
        self.notes.write_text("Prepared notes")

    def snapshot(self, backend):
        return publish.snapshot(backend, NEW, self.output, self.backup, self.notes)

    def execute(self, backend, artifact="123"):
        return publish.publish(backend, NEW, "123.2", artifact, self.output, self.backup, self.notes)

    def journal(self):
        return json.loads((self.backup / "journal.json").read_text())

    def assert_restored(self, backend, original, contents):
        state = backend.state()
        self.assertEqual(state["tag"], original["tag"])
        self.assertEqual(state["latest"], original["latest"])
        if original["release"]:
            for key in publish.METADATA:
                self.assertEqual(state["release"].get(key), original["release"].get(key))
            publish.delivery.validate_uploaded(state["release"], {
                name: hashlib.sha256(data).hexdigest() for name, data in contents.items()
            })
            self.assertEqual({a["label"] for a in state["release"]["assets"]}, {"previous label"})
        else:
            self.assertIsNone(state["release"])

    def test_success_uses_annotated_owner_and_exact_lease_after_backup(self):
        backend = FakeNative()
        self.snapshot(backend)
        self.assertFalse(any("push" in c for c in backend.calls))
        self.execute(backend)
        self.assertEqual(backend.tag()["commit"], NEW)
        self.assertNotEqual(backend.tag()["object"], NEW)
        self.assertEqual(backend.latest, 7)
        self.assertEqual(self.journal()["status"], "published")
        pushes = [c for c in backend.calls if "push" in c]
        self.assertIn(f"--force-with-lease={publish.REF}:{'c' * 40}", pushes[0])
        publish.delivery.validate_uploaded(backend.release, publish.hashes(self.output))

    def test_ordinary_and_applied_write_failures_restore_existing_delivery(self):
        for stage, after in [("push", False), ("push", True), ("upload", False),
                             ("upload-deleted", False), ("upload", True),
                             ("edit", False), ("edit", True)]:
            with self.subTest(stage=stage, after=after), tempfile.TemporaryDirectory() as tmp:
                self.backup = Path(tmp) / "backup"
                backend = FakeNative(unrelated_latest=True)
                original, contents = copy.deepcopy(backend.state()), copy.deepcopy(backend.files)
                self.snapshot(backend)
                backend.fail(stage, after)
                with self.assertRaises(subprocess.CalledProcessError):
                    self.execute(backend)
                self.assert_restored(backend, original, contents)
                self.assertIn(self.journal()["status"], {"restored", "previous-state-unchanged"})

    def test_first_publication_failures_preserve_unrelated_latest(self):
        for stage, after in [("create", False), ("create", True), ("upload-deleted", False),
                             ("edit", False), ("edit", True)]:
            with self.subTest(stage=stage, after=after), tempfile.TemporaryDirectory() as tmp:
                self.backup = Path(tmp) / "backup"
                backend = FakeNative(existing=False, unrelated_latest=True)
                original = copy.deepcopy(backend.state())
                self.snapshot(backend)
                backend.fail(stage, after)
                with self.assertRaises(subprocess.CalledProcessError):
                    self.execute(backend)
                self.assert_restored(backend, original, {})
                self.assertEqual(self.journal()["status"], "restored")

    def test_new_draft_waits_for_visibility_before_any_upload(self):
        backend = FakeNative(existing=False, unrelated_latest=True)
        self.snapshot(backend)
        backend.hide_after_create = 2

        def check_wait(delay):
            self.assertEqual(backend.upload_count, 0)
            self.assertEqual(self.journal()["release_id"], 8)
            self.assertEqual(self.journal()["status"], "waiting-for-draft")

        with mock.patch.object(publish.time, "sleep", side_effect=check_wait) as sleep:
            self.execute(backend)
        self.assertEqual(sleep.call_args_list, [mock.call(1), mock.call(2)])
        self.assertEqual(self.journal()["status"], "published")
        self.assertEqual(backend.upload_count, len(publish.NAMES))

    def test_new_draft_visibility_timeout_fails_closed_and_recovers_visible_draft(self):
        backend = FakeNative(existing=False, unrelated_latest=True)
        original = copy.deepcopy(backend.state())
        self.snapshot(backend)
        backend.hide_after_create = 5
        with mock.patch.object(publish.time, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "did not become visible"):
                self.execute(backend)
        self.assertEqual(sleep.call_args_list, [mock.call(1), mock.call(2), mock.call(4), mock.call(8)])
        self.assertEqual(backend.upload_count, 0)
        self.assert_restored(backend, original, {})
        self.assertEqual(self.journal()["status"], "restored")
        self.assertEqual(self.journal()["release_id"], 8)

    def test_new_draft_persistently_absent_requires_manual_recovery_without_mutation(self):
        backend = FakeNative(existing=False, unrelated_latest=True)
        self.snapshot(backend)
        backend.hide_after_create = 99
        with mock.patch.object(publish.time, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "did not become visible"):
                self.execute(backend)
        self.assertEqual(sleep.call_args_list, [mock.call(1), mock.call(2), mock.call(4), mock.call(8)])
        journal = self.journal()
        self.assertEqual(backend.upload_count, 0)
        self.assertEqual(journal["status"], "manual-recovery-required")
        self.assertEqual(journal["release_id"], 8)
        self.assertEqual(backend.raw_tag, journal["publisher_tag"])
        self.assertEqual(backend.objects[backend.raw_tag], NEW)
        self.assertEqual(backend.release["id"], 8)
        self.assertEqual(len([call for call in backend.calls if "push" in call]), 1)
        self.assertFalse(any("DELETE" in call for call in backend.calls))

    def test_new_draft_foreign_id_is_rejected_without_polling_or_upload(self):
        backend = FakeNative(existing=False)
        self.snapshot(backend)
        read_state = backend.state

        def foreign_state():
            if backend.release:
                backend.release["id"] = 99
            return read_state()

        with mock.patch.object(backend, "state", side_effect=foreign_state), \
                mock.patch.object(publish.time, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "release identity changed"):
                self.execute(backend)
        sleep.assert_not_called()
        self.assertEqual(backend.upload_count, 0)
        self.assertEqual(backend.release["id"], 99)
        self.assertEqual(self.journal()["release_id"], 8)
        self.assertEqual(self.journal()["status"], "manual-recovery-required")

    def test_hidden_new_draft_does_not_tolerate_disappearing_previous_latest(self):
        backend = FakeNative(existing=False, unrelated_latest=True)
        self.snapshot(backend)
        backend.hide_after_create = 1
        read_state = backend.state

        def lose_latest():
            if backend.release:
                backend.latest = None
            return read_state()

        with mock.patch.object(backend, "state", side_effect=lose_latest), \
                mock.patch.object(publish.time, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "latest release changed"):
                self.execute(backend)
        sleep.assert_not_called()
        self.assertEqual(backend.upload_count, 0)
        self.assertEqual(self.journal()["release_id"], 8)
        self.assertEqual(self.journal()["status"], "manual-recovery-required")

    def test_verification_failure_restores_before_editing_release_metadata(self):
        backend = FakeNative()
        original, contents = copy.deepcopy(backend.state()), copy.deepcopy(backend.files)
        self.snapshot(backend)
        backend.bad_verification = True
        with self.assertRaisesRegex(ValueError, "incomplete upload"):
            self.execute(backend)
        self.assert_restored(backend, original, contents)
        self.assertEqual(self.journal()["status"], "restored")

    def test_successful_first_publication_marks_latest_and_keeps_unique_owner(self):
        backend = FakeNative(existing=False, unrelated_latest=True)
        self.snapshot(backend)
        self.execute(backend)
        self.assertEqual(backend.latest, 8)
        self.assertFalse(backend.release["draft"])
        self.assertEqual(self.journal()["status"], "published")

    def test_no_mutation_without_uploaded_backup_or_with_stale_snapshot(self):
        backend = FakeNative()
        self.snapshot(backend)
        with self.assertRaisesRegex(ValueError, "artifact ID"):
            self.execute(backend, "")
        backend.release["body"] = "manual edit"
        with self.assertRaisesRegex(ValueError, "stale"):
            self.execute(backend)
        self.assertFalse(any("push" in c for c in backend.calls))

    def test_incompatible_inventory_and_unverifiable_backup_stop_before_mutation(self):
        for change in [lambda b: b.release["assets"].pop(),
                       lambda b: b.release.update(draft=True),
                       lambda b: b.release.update(immutable=True),
                       lambda b: b.release["assets"][0].update(digest=None),
                       lambda b: b.files.update({"PUBLISH_INFO.json": b"tampered"})]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as tmp:
                self.backup = Path(tmp) / "backup"
                backend = FakeNative()
                change(backend)
                with self.assertRaises(ValueError):
                    self.snapshot(backend)
                self.assertFalse(any("push" in c for c in backend.calls))

    def test_newer_publisher_same_source_is_never_rolled_back(self):
        backend = FakeNative()
        self.snapshot(backend)

        def advance(b):
            b.raw_tag = "d" * 40
            b.objects[b.raw_tag] = NEW

        backend.fail("upload", after=True, external=advance)
        with self.assertRaises(subprocess.CalledProcessError):
            self.execute(backend)
        self.assertEqual(backend.raw_tag, "d" * 40)
        self.assertEqual(self.journal()["status"], "manual-recovery-required")
        self.assertEqual(len([c for c in backend.calls if "push" in c]), 1)

    def test_manual_metadata_or_unknown_asset_bytes_prevent_recovery_overwrite(self):
        for change in [lambda b: b.release.update(body="human changed notes"),
                       lambda b: b.release["assets"][0].update(digest="sha256:" + "f" * 64)]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as tmp:
                self.backup = Path(tmp) / "backup"
                backend = FakeNative()
                self.snapshot(backend)
                backend.fail("upload", after=True, external=change)
                with self.assertRaises(subprocess.CalledProcessError):
                    self.execute(backend)
                self.assertEqual(self.journal()["status"], "manual-recovery-required")
                self.assertFalse(any(c[:3] == ("gh", "release", "upload") and "/assets/" in c[4]
                                     for c in backend.calls))

    def test_api_failure_is_not_missing_release_and_backup_tampering_is_rejected(self):
        backend = FakeNative()
        backend.fail("read")
        with self.assertRaises(subprocess.CalledProcessError):
            self.snapshot(backend)
        self.snapshot(backend)
        (self.backup / "assets" / "SHA256SUMS").write_text("tampered")
        with self.assertRaisesRegex(ValueError, "backup changed"):
            self.execute(backend)
        self.assertFalse(any("push" in c for c in backend.calls))

    def test_recovery_failure_preserves_evidence_and_does_not_claim_restored(self):
        backend = FakeNative()
        self.snapshot(backend)

        def fail_restore(b):
            b.fail("restore-upload")

        backend.fail("upload-deleted", external=fail_restore)
        with self.assertRaises(subprocess.CalledProcessError):
            self.execute(backend)
        self.assertEqual(self.journal()["status"], "manual-recovery-required")
        self.assertTrue((self.backup / "state.json").is_file())
        self.assertEqual(len([c for c in backend.calls if "push" in c]), 1)


if __name__ == "__main__":
    unittest.main()
