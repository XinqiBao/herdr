"""Guarded replacement of one mutable fork release using native gh/Git commands."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

if __package__:
    from . import fork_release as delivery
else:
    import fork_release as delivery

REPO = delivery.REPOSITORY
TAG = delivery.TAG
REF = f"refs/tags/{TAG}"
BACKUP = Path("release-backup")
METADATA = ("tag_name", "target_commitish", "name", "body", "draft", "prerelease")
NAMES = set(delivery.ARTIFACTS) | {f"{n}.BUILD_INFO.txt" for n in delivery.ARTIFACTS} | {
    "SHA256SUMS", "PUBLISH_INFO.json",
}
require = delivery.require


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def hashes(directory):
    require({p.name for p in directory.iterdir()} == NAMES, "incompatible local asset inventory")
    require(all(p.is_file() and not p.is_symlink() for p in directory.iterdir()),
            "regular asset files required")
    return {p.name: delivery.digest(p) for p in directory.iterdir()}


def fingerprint(state):
    """Ignore counters/timestamps changed by downloading; retain mutation identity."""
    release = state["release"]
    return {
        "tag": state["tag"], "latest": state["latest"],
        "release": None if release is None else {
            "id": release["id"], "immutable": release.get("immutable", False),
            **{k: release.get(k) for k in METADATA},
            "assets": sorted((a["id"], a["name"], a["state"], a.get("digest"),
                              a["size"], a.get("label")) for a in release["assets"]),
        },
    }


class Native:
    def run(self, *args, data=None):
        return subprocess.run(args, input=data, text=True, check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    def api(self, path, method="GET", payload=None, missing=False):
        args = ["gh", "api", f"repos/{REPO}/{path}", "--method", method]
        if payload is not None:
            args += ["--input", "-"]
        try:
            result = self.run(*args, data=None if payload is None else json.dumps(payload))
        except subprocess.CalledProcessError as error:
            if missing and "(HTTP 404)" in (error.stderr or ""):
                return None
            raise
        return json.loads(result) if result.strip() else None

    def tag(self):
        refs = {line.split()[1]: line.split()[0] for line in self.run(
            "git", "ls-remote", "origin", REF, REF + "^{}"
        ).splitlines()}
        return None if REF not in refs else {
            "object": refs[REF], "commit": refs.get(REF + "^{}", refs[REF]),
        }

    def state(self):
        pages = json.loads(self.run("gh", "api", "--paginate", "--slurp",
                                    f"repos/{REPO}/releases?per_page=100"))
        matches = [r for page in pages for r in page if r["tag_name"] == TAG]
        require(len(matches) <= 1, "ambiguous rolling release")
        latest = self.api("releases/latest", missing=True)
        return {"tag": self.tag(), "release": matches[0] if matches else None,
                "latest": latest["id"] if latest else None}

    def main(self):
        return self.run("git", "ls-remote", "origin", "refs/heads/xbao/main").split()[0]

    def download(self, directory):
        self.run("gh", "release", "download", TAG, "--repo", REPO, "--dir", str(directory))

    def retain_tag(self, tag, directory):
        if tag:
            self.run("git", "fetch", "--no-tags", "origin", REF)
            require(self.run("git", "rev-parse", "FETCH_HEAD").strip() == tag["object"],
                    "tag moved while retaining previous object")
            kind = self.run("git", "cat-file", "-t", tag["object"]).strip()
            # Preserve an annotated object's exact bytes for native manual recovery.
            if kind == "tag":
                (directory / "previous-tag.txt").write_text(
                    self.run("git", "cat-file", "tag", tag["object"]))

    def new_tag(self, commit, owner):
        return self.run("git", "mktag", data=(
            f"object {commit}\ntype commit\ntag {TAG}\n"
            f"tagger Fork publisher <fork-publisher@users.noreply.github.com> {int(time.time())} +0000\n\n"
            f"Actions publisher {owner}\n"
        )).strip()

    def push_tag(self, expected, replacement):
        self.run("git", "-c", "credential.helper=!gh auth git-credential", "push",
                 f"--force-with-lease={REF}:{expected or ''}", "origin", f"{replacement or ''}:{REF}")

    def upload(self, path, label=None):
        argument = str(path) if not label else f"{path}#{label}"
        self.run("gh", "release", "upload", TAG, argument, "--repo", REPO, "--clobber")


def snapshot(backend, commit, output=delivery.OUTPUT, directory=BACKUP,
             notes=Path("release-notes.md")):
    expected = hashes(output)
    require(backend.main() == commit, "xbao/main moved from validated source")
    state = backend.state()
    release, tag = state["release"], state["tag"]
    require(bool(release) == bool(tag), "orphan tag/release requires manual reconciliation")
    if release:
        require(not release.get("immutable") and not release["draft"],
                "previous release must be published and mutable")
        assets = release["assets"]
        require(len(assets) == len(NAMES) and {a["name"] for a in assets} == NAMES,
                "incompatible previous asset inventory")
        previous = {}
        for asset in assets:
            checksum = asset.get("digest", "") or ""
            require(re.fullmatch(r"sha256:[0-9a-f]{64}", checksum)
                    and asset["state"] == "uploaded", "incomplete previous asset provenance")
            previous[asset["name"]] = checksum.removeprefix("sha256:")
    else:
        previous = {}
    directory.mkdir()
    if release:
        assets_dir = directory / "assets"
        assets_dir.mkdir()
        backend.download(assets_dir)
        require(hashes(assets_dir) == previous, "previous download digest mismatch")
        info = json.loads((assets_dir / "PUBLISH_INFO.json").read_text())
        require(info["source_commit"] == tag["commit"], "previous source/tag mismatch")
    backend.retain_tag(tag, directory)
    require(fingerprint(backend.state()) == fingerprint(state), "previous state moved during backup")
    backup = {"previous": state, "previous_digests": previous,
              "source_commit": commit, "delivery_digests": expected,
              "notes_sha256": delivery.digest(notes)}
    write_json(directory / "state.json", backup)
    return backup


def publish(backend, commit, owner, artifact_id, output=delivery.OUTPUT,
            directory=BACKUP, notes=Path("release-notes.md")):
    require(re.fullmatch(r"[0-9]+", artifact_id or ""), "uploaded backup artifact ID required")
    backup = json.loads((directory / "state.json").read_text())
    previous = backup["previous"]
    require(backup["source_commit"] == commit and hashes(output) == backup["delivery_digests"]
            and delivery.digest(notes) == backup["notes_sha256"], "prepared delivery changed")
    if previous["release"]:
        require(hashes(directory / "assets") == backup["previous_digests"], "backup changed")
    require(backend.main() == commit, "xbao/main moved from validated source")
    require(fingerprint(backend.state()) == fingerprint(previous), "backup is stale")
    new_tag = backend.new_tag(commit, owner)
    old_tag = previous["tag"]["object"] if previous["tag"] else None
    marker = f"<!-- xbao-publisher:{owner} -->"
    body = marker + "\n" + notes.read_text()
    release_id = previous["release"]["id"] if previous["release"] else None
    journal = {"source_commit": commit, "owner": owner, "backup_artifact_id": artifact_id,
               "previous_tag": old_tag, "publisher_tag": new_tag, "status": "starting"}

    def record(status, error=None):
        journal.update(status=status, release_id=release_id)
        if error is not None:
            journal["error"] = str(error)
        write_json(directory / "journal.json", journal)

    final_metadata = {
        "tag_name": TAG, "target_commitish": commit, "name": "Herdr · xbao latest",
        "body": body, "draft": False, "prerelease": False,
    }

    def owned(allow_absent=False):
        state = backend.state()
        require(state["tag"] == {"object": new_tag, "commit": commit},
                "publisher tag ownership lost; manual recovery required")
        current = state["release"]
        if release_id is not None and not (allow_absent and current is None):
            require(current and current["id"] == release_id, "release identity changed")
        if current:
            original = previous["release"]
            allowed = [final_metadata, dict(final_metadata, draft=True)]
            if original:
                allowed.append({k: original.get(k) for k in METADATA})
            require(not current.get("immutable") and
                    {k: current.get(k) for k in METADATA} in allowed,
                    "release metadata changed outside publisher")
        allowed_latest = {previous["latest"], release_id}
        if allow_absent and current is None:
            allowed_latest.add(None)
        require(state["latest"] in allowed_latest, "latest release changed")
        return state

    def recover():
        nonlocal release_id
        # A create request can have succeeded even when its response was lost.
        if release_id is None:
            discovered = backend.state()["release"]
            if discovered and discovered.get("body") == body:
                release_id = discovered["id"]
        state = owned()
        current = state["release"]
        if previous["release"]:
            # Refuse unexpected manual edits or another publisher's bytes/metadata.
            original = previous["release"]
            allowed_metadata = [{k: original.get(k) for k in METADATA}, final_metadata]
            require({k: current.get(k) for k in METADATA} in allowed_metadata,
                    "release metadata changed outside publisher")
            require(state["latest"] in {previous["latest"], release_id}, "latest release changed")
            require(len({a["name"] for a in current["assets"]}) == len(current["assets"]),
                    "duplicate asset names during recovery")
            for asset in current["assets"]:
                name = asset["name"]
                require(name in NAMES and (asset["state"] in {"new", "starter"} or
                        asset.get("digest") in {f"sha256:{backup['previous_digests'][name]}",
                                                f"sha256:{backup['delivery_digests'][name]}"}),
                        "asset state changed outside publisher")
            for asset in original["assets"]:
                owned()
                backend.upload(directory / "assets" / asset["name"], asset.get("label"))
            owned()
            restored = {k: original.get(k) for k in METADATA}
            if previous["latest"] == release_id:
                restored["make_latest"] = "true"
            elif previous["latest"] is None:
                restored["make_latest"] = "false"
            backend.api(f"releases/{release_id}", "PATCH", restored)
        elif current:
            require(marker in (current.get("body") or ""), "unknown new release; manual recovery required")
            owned()
            backend.api(f"releases/{current['id']}", "DELETE")
        if previous["latest"] is not None and previous["latest"] != release_id:
            state = owned(allow_absent=True)
            # An absent release is expected after deleting a new failed delivery.
            require(state["latest"] in {previous["latest"], release_id, None}, "latest release changed")
            if state["latest"] != previous["latest"]:
                backend.api(f"releases/{previous['latest']}", "PATCH", {"make_latest": "true"})
        state = owned() if previous["release"] else backend.state()
        require(state["tag"] == {"object": new_tag, "commit": commit}, "publisher tag ownership lost")
        if previous["release"]:
            delivery.validate_uploaded(state["release"], backup["previous_digests"])
            require(all(state["release"].get(k) == previous["release"].get(k) for k in METADATA),
                    "restored metadata mismatch")
        else:
            require(state["release"] is None, "new release was not removed")
        require(state["latest"] == previous["latest"], "previous latest pointer not restored")
        backend.push_tag(new_tag, old_tag)

    record("starting")
    try:
        backend.push_tag(old_tag, new_tag)
        owned()
        if release_id is None:
            created = backend.api("releases", "POST", {
                "tag_name": TAG, "target_commitish": commit, "name": "Herdr · xbao latest",
                "body": body, "draft": True, "prerelease": False, "make_latest": "false",
            })
            release_id = created["id"]
            record("waiting-for-draft")
            # The create response can precede visibility in the release listing.
            # Retry only absence; owned() still rejects foreign identity/state.
            for delay in (0, 1, 2, 4, 8):
                if delay:
                    time.sleep(delay)
                state = owned(allow_absent=True)
                require(state["latest"] in {previous["latest"], release_id}, "latest release changed")
                if state["release"] is not None:
                    break
            else:
                raise ValueError("new draft did not become visible before upload")
        record("uploading")
        for name in sorted(NAMES):
            owned()
            backend.upload(output / name)
        state = owned()
        delivery.validate_uploaded(state["release"], backup["delivery_digests"])
        backend.api(f"releases/{release_id}", "PATCH", {
            "tag_name": TAG, "target_commitish": commit, "name": "Herdr · xbao latest",
            "body": body, "draft": False, "prerelease": False, "make_latest": "true",
        })
        state = owned()
        delivery.validate_uploaded(state["release"], backup["delivery_digests"])
        require(state["latest"] == release_id and state["release"]["body"] == body
                and not state["release"]["draft"], "publication metadata mismatch")
        record("published")
    except Exception as error:
        record("failed", error)
        try:
            # A failed push can mean either rejection or an applied write with a lost response.
            if fingerprint(backend.state()) == fingerprint(previous):
                record("previous-state-unchanged", error)
            else:
                recover()
                record("restored", error)
        except Exception as recovery_error:
            journal["recovery_error"] = str(recovery_error)
            record("manual-recovery-required", error)
            print(f"Recovery stopped: {recovery_error}. Use the retained backup artifact.", file=sys.stderr)
        raise


def main(mode):
    require(os.environ["GITHUB_REPOSITORY"] == REPO, "personal fork only")
    commit = os.environ["GITHUB_SHA"]
    require(re.fullmatch(r"[0-9a-f]{40}", commit), "invalid source SHA")
    backend = Native()
    require(backend.run("git", "rev-parse", "HEAD").strip() == commit, "checkout source mismatch")
    require(backend.run("git", "remote", "get-url", "origin").strip()
            in {f"https://github.com/{REPO}", f"https://github.com/{REPO}.git"},
            "unexpected origin")
    if mode == "snapshot":
        snapshot(backend, commit)
    else:
        require(mode == "publish", "expected snapshot or publish")
        owner = f"{os.environ['GITHUB_RUN_ID']}.{os.environ['GITHUB_RUN_ATTEMPT']}"
        publish(backend, commit, owner, os.environ.get("BACKUP_ARTIFACT_ID"))


if __name__ == "__main__":
    main(sys.argv[1])
