"""Verify and assemble this fork's complete Actions-built rolling delivery."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import zipfile

if __package__:
    from . import package_windows_conpty as conpty
else:
    import package_windows_conpty as conpty

REPOSITORY = "XinqiBao/herdr"
TAG = "xbao-latest"
ARTIFACTS = {
    "herdr-linux-x86_64": "x86_64-unknown-linux-musl",
    "herdr-linux-aarch64": "aarch64-unknown-linux-musl",
    "herdr-macos-x86_64": "x86_64-apple-darwin",
    "herdr-macos-aarch64": "aarch64-apple-darwin",
    "herdr-windows-x86_64.zip": "x86_64-pc-windows-msvc",
}
SUFFIX = "-ReleaseFast-simd-true"
OUTPUT = Path("release-assets")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def verify_base(path=Path("fork-base.json")):
    base = json.loads(path.read_text())
    require(base["repository"] == "herdrdev/herdr", "canonical upstream required")
    require(re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", base["tag"]), "invalid stable base tag")
    require(re.fullmatch(r"[0-9a-f]{40}", base["commit"]), "invalid base SHA")
    ref = f"refs/tags/{base['tag']}"
    refs = dict((line.split()[1], line.split()[0]) for line in command(
        "git", "ls-remote", "https://github.com/herdrdev/herdr.git", ref, ref + "^{}"
    ).splitlines())
    require(ref in refs and refs.get(ref + "^{}", refs.get(ref)) == base["commit"],
            "canonical upstream base tag mismatch")
    command("git", "merge-base", "--is-ancestor", base["commit"], "HEAD")
    return base


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def validate_artifact(directory, name, commit, run_url, attempt):
    require(directory.is_dir() and not directory.is_symlink(), "missing artifact directory")
    require({p.name for p in directory.iterdir()} == {name, "BUILD_INFO.txt"},
            f"unexpected artifact files: {name}")
    binary, info = directory / name, directory / "BUILD_INFO.txt"
    require(all(p.is_file() and not p.is_symlink() for p in (binary, info)),
            "expected regular artifact files")
    fields = {}
    for line in info.read_text(encoding="utf-8-sig").splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            require(key not in fields, f"duplicate BUILD_INFO key: {key}")
            fields[key] = value
    expected = {
        "commit": commit,
        "repository": REPOSITORY,
        "run_url": run_url,
        "run_attempt": str(attempt),
        "build_channel": "fork",
        "build_id": f"xbao.{commit[:7]}",
        "target": ARTIFACTS[name],
        "libghostty_vt_optimize": "ReleaseFast",
        "libghostty_vt_simd": "true",
        "sha256": f"{digest(binary)}  {name}",
    }
    require(all(fields.get(k) == v for k, v in expected.items()),
            f"artifact source, settings or checksum mismatch: {name}")


def validate_windows(path, metadata):
    # Reuse the upstream package contract without extracting untrusted zip paths.
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        require(len(names) == len(set(names)), "duplicate Windows archive entries")
        require(set(names) == conpty.expected_stage_files(metadata, "x86_64"),
                "Windows archive layout differs from the upstream bundle")
        require(archive.read(conpty.MARKER_PATH.as_posix()) == conpty.marker_data(metadata, "x86_64"),
                "Windows ConPTY marker mismatch")
        conpty.validate_static_msvc_runtime(archive.read("herdr.exe"), "herdr.exe")
        for item in metadata["bundles"]["x86_64"]["files"] + metadata["notices"]:
            require(hashlib.sha256(archive.read(item["destination"])).hexdigest() == item["sha256"],
                    f"Windows bundle checksum mismatch: {item['destination']}")


def validate_uploaded(release, expected):
    require(release is not None, "rolling release does not exist")
    assets = release["assets"]
    actual = {a["name"]: a for a in assets}
    require(len(actual) == len(assets) and actual.keys() == expected.keys(),
            "uploaded asset set differs from the complete verified delivery")
    for name, checksum in expected.items():
        require(actual[name]["state"] == "uploaded", f"incomplete upload: {name}")
        require(actual[name].get("digest") == f"sha256:{checksum}",
                f"uploaded checksum mismatch: {name}")


def main(mode):
    require(os.environ["GITHUB_REPOSITORY"] == REPOSITORY, "personal fork only")
    commit = os.environ["GITHUB_SHA"]
    require(re.fullmatch(r"[0-9a-f]{40}", commit), "invalid source SHA")
    require(command("git", "rev-parse", "HEAD") == commit, "checkout source mismatch")
    require(mode == "prepare", "expected prepare")
    base = verify_base()
    run_url = f"https://github.com/{REPOSITORY}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    attempt = os.environ["GITHUB_RUN_ATTEMPT"]
    downloads = Path("downloaded-artifacts")
    expected_dirs = {name.removesuffix(".zip") + SUFFIX for name in ARTIFACTS}
    require({p.name for p in downloads.iterdir()} == expected_dirs, "incomplete platform set")
    for name in ARTIFACTS:
        validate_artifact(downloads / (name.removesuffix(".zip") + SUFFIX),
                          name, commit, run_url, attempt)
    validate_windows(downloads / ("herdr-windows-x86_64" + SUFFIX) / "herdr-windows-x86_64.zip",
                     conpty.load_metadata(conpty.DEFAULT_METADATA))
    require(not OUTPUT.exists(), "release output already exists")
    OUTPUT.mkdir()
    for name in ARTIFACTS:
        source = downloads / (name.removesuffix(".zip") + SUFFIX)
        shutil.copyfile(source / name, OUTPUT / name)
        shutil.copyfile(source / "BUILD_INFO.txt", OUTPUT / f"{name}.BUILD_INFO.txt")
    (OUTPUT / "SHA256SUMS").write_text("".join(
        f"{digest(OUTPUT / name)}  {name}\n" for name in sorted(ARTIFACTS)))
    (OUTPUT / "PUBLISH_INFO.json").write_text(json.dumps({
        "source_commit": commit, "upstream_base": base["tag"],
        "upstream_base_commit": base["commit"], "upstream_repository": base["repository"],
        "run_url": run_url, "run_attempt": attempt,
    }, indent=2) + "\n")
    Path("release-notes.md").write_text(
        f"Rolling fork delivery of Herdr {base['tag']} (`{base['commit']}`); source `{commit}`.\n\n"
        f"All five platform builds and Linux/macOS/Windows checks: {run_url} "
        f"(attempt {attempt}). Production builds use ReleaseFast and SIMD.\n\n"
        "Verify downloads with SHA256SUMS; BUILD_INFO and PUBLISH_INFO.json record provenance. "
        "This release/tag is replaced on subsequent deliveries. Asset replacement on an "
        "existing release is not atomic; download after the publisher run succeeds. "
        "Failed or cancelled runs can require manual recovery from the run's last-good "
        "backup artifact; see FORK_WORKFLOW.md.\n\n"
        + Path("PATCHES.md").read_text()
    )


if __name__ == "__main__":
    main(sys.argv[1])
