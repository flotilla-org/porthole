#!/usr/bin/env python3
"""Build a credential-free Darwin candidate from a clean, exact checkout."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[2]
NAME = "porthole-candidate-darwin-aarch64"
BINARIES = ("PortholeHelper", "portholed", "porthole", "jackstay-bridge")
RESOURCES = (
    "Contents/Info.plist",
    "Contents/Resources/icon.png",
    "Contents/Library/LaunchAgents/work.flotilla.porthole.daemon.plist",
)


def output(*args, cwd=ROOT, env=None):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()


def run(*args, cwd=ROOT, env=None):
    subprocess.run(args, cwd=cwd, env=env, check=True)


def digest(path):
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def resolved_jackstay(metadata):
    packages = [p for p in metadata["packages"] if p["name"] in ("jackstay", "jackstay-graph")]
    if sorted(p["name"] for p in packages) != ["jackstay", "jackstay-graph"]:
        raise ValueError("expected exactly one jackstay and jackstay-graph package")
    sources = {p["source"] for p in packages}
    if len(sources) != 1:
        raise ValueError("jackstay and jackstay-graph must resolve to the same source")
    match = re.fullmatch(
        r"git\+https://github.com/flotilla-org/jackstay\?rev=([0-9a-f]{40})#([0-9a-f]{40})",
        sources.pop() or "",
    )
    if not match or match[1] != match[2]:
        raise ValueError("Jackstay must resolve to its exact canonical 40-character pin")
    package = next(p for p in packages if p["name"] == "jackstay")
    return match[1], Path(package["manifest_path"]).parents[2]


def compatibility(source):
    header = (source / "crates/jackstay/include/capture_transfer.h").read_text()
    wire = (source / "crates/jackstay-bridge/src/wire.rs").read_text()
    def number(pattern, text):
        match = re.search(pattern, text)
        if not match:
            raise ValueError("could not read Jackstay compatibility constants")
        return int(match[1])
    return {
        "jackstay_c_abi": {
            "major": number(r"#define\s+FT_ABI_VERSION_MAJOR\s+(\d+)", header),
            "minor": number(r"#define\s+FT_ABI_VERSION_MINOR\s+(\d+)", header),
        },
        "jackstay_bridge_wire": number(r"pub const VERSION: u8 = (\d+);", wire),
    }


def inventory(app):
    expected = set(RESOURCES) | {f"Contents/MacOS/{name}" for name in BINARIES}
    files = {}
    for path in sorted(app.rglob("*")):
        relative = path.relative_to(app).as_posix()
        if path.is_symlink():
            raise ValueError(f"unexpected symlink in candidate: {relative}")
        if path.is_dir():
            continue
        if relative not in expected:
            raise ValueError(f"unexpected candidate entry: {relative}")
        if not path.is_file():
            raise ValueError(f"invalid file type in candidate: {relative}")
        executable = relative.startswith("Contents/MacOS/")
        if executable and not path.stat().st_mode & 0o111:
            raise ValueError(f"candidate binary is not executable: {relative}")
        files[f"Porthole.app/{relative}"] = {
            "sha256": digest(path), "size_bytes": path.stat().st_size,
            "mode": "0755" if executable else "0644",
        }
    if {p.removeprefix("Porthole.app/") for p in files} != expected:
        raise ValueError("candidate is missing required bundle files")
    return files


def verify_linkage(app):
    for name in BINARIES:
        binary = app / "Contents/MacOS" / name
        if output("/usr/bin/lipo", "-archs", str(binary)) != "arm64":
            raise ValueError(f"expected arm64 only: {name}")
        dependencies = output("/usr/bin/otool", "-L", str(binary)).splitlines()[1:]
        for line in dependencies:
            dependency = line.strip().split(" (", 1)[0]
            if not dependency.startswith(("/usr/lib/", "/System/Library/")):
                raise ValueError(f"non-system runtime dependency in {name}: {dependency}")
        # Absolute build paths are harmless in debug records but not LC_RPATH.
        load_commands = output("/usr/bin/otool", "-l", str(binary))
        for path in re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset", load_commands):
            if not path.startswith(("@executable_path/", "@loader_path/", "/usr/lib/", "/System/Library/")):
                raise ValueError(f"non-relocatable runtime search path in {name}: {path}")


def remove_swift_toolchain_rpaths(app):
    # SwiftBuild can inject its own toolchain rpath even with swiftc's
    # -no-toolchain-stdlib-rpath. Remove only the active toolchain's fallback;
    # verify_linkage still rejects non-system dependencies and other paths.
    helper = app / "Contents/MacOS/PortholeHelper"
    toolchain_lib = Path(output("xcrun", "--find", "swiftc")).parents[1] / "lib"
    commands = output("/usr/bin/otool", "-l", str(helper))
    for path in re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset", commands):
        if re.fullmatch(re.escape(str(toolchain_lib)) + r"/swift(?:-[0-9.]+)?/macosx", path):
            run("/usr/bin/install_name_tool", "-delete_rpath", path, str(helper))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New output directory (must not exist)")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_sha):
        parser.error("--source-sha must be a full lowercase commit")
    if (platform.system(), platform.machine()) != ("Darwin", "arm64"):
        parser.error("requires Darwin arm64")
    if output("git", "rev-parse", "HEAD") != args.source_sha or output("git", "status", "--porcelain"):
        parser.error("requires a clean checkout at --source-sha")
    identities = output("/usr/bin/security", "find-identity", "-v", "-p", "codesigning")
    if not re.search(r"^\s*0 valid identities found\s*$", identities, re.MULTILINE):
        parser.error("candidate worker must have no signing identities")
    if args.output.exists():
        parser.error("output directory already exists")
    # xtask's bundle paths are relative to target/. Refuse overrides rather
    # than accidentally packaging stale binaries from a different directory.
    env = os.environ.copy()
    for name in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "JACKSTAY_BRIDGE_BIN"):
        env.pop(name, None)
    env["CARGO_INCREMENTAL"] = "0"
    metadata = json.loads(output("cargo", "metadata", "--format-version", "1", "--locked", env=env))
    if Path(metadata["target_directory"]).resolve() != (ROOT / "target").resolve():
        parser.error("cargo configuration must use the local target directory")
    revision, jackstay = resolved_jackstay(metadata)
    versions = compatibility(jackstay)
    bridge_target = ROOT / "target/fleet-jackstay" / revision
    run("cargo", "build", "--manifest-path", str(jackstay / "Cargo.toml"), "--locked", "--release",
        "-p", "jackstay-bridge", "--features", "backend-macos", "--target-dir", str(bridge_target), env=env)
    env["JACKSTAY_BRIDGE_BIN"] = str(bridge_target / "release/jackstay-bridge")
    # A fresh module cache prevents serialized Swift/Clang modules referring
    # to a previous VM's checkout. Only Cargo outputs are shared between runs.
    with tempfile.TemporaryDirectory(prefix="porthole-swift-") as cache:
        env["CLANG_MODULE_CACHE_PATH"] = str(Path(cache) / "clang")
        env["SWIFT_MODULECACHE_PATH"] = str(Path(cache) / "modules")
        env["PORTHOLE_SWIFT_SCRATCH_PATH"] = str(Path(cache) / "build")
        run("cargo", "run", "--locked", "-p", "xtask", "--", "bundle", "--platform", "macos",
            "--release", "--unsigned", env=env)
    app = ROOT / "target/release/Porthole.app"
    remove_swift_toolchain_rpaths(app)
    files = inventory(app)
    verify_linkage(app)
    args.output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="porthole-package-") as temporary:
        bundle = Path(temporary) / NAME
        bundle.mkdir()
        shutil.copytree(app, bundle / "Porthole.app")
        for relative, entry in files.items():
            (bundle / relative).chmod(int(entry["mode"], 8))
        manifest = {
            "schema_version": 1, "kind": "porthole-fleet-candidate", "state": "requires-central-signing",
            "platform": "darwin-aarch64", "sources": {"porthole": args.source_sha, "jackstay": revision},
            "compatibility": versions, "files": files,
            "toolchain": {"rustc": output("rustc", "--version"), "xcode": output("xcodebuild", "-version")},
        }
        write_json(bundle / "candidate.json", manifest)
        archive = args.output / f"{NAME}.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(bundle, arcname=NAME)
    archive_sha = digest(archive)
    write_json(args.output / f"{NAME}.json", {
        **{key: value for key, value in manifest.items() if key != "files"},
        "artifact": archive.name, "sha256": archive_sha, "size_bytes": archive.stat().st_size,
    })
    (args.output / f"{NAME}.sha256").write_text(f"{archive_sha}  {archive.name}\n")
    print(archive)


if __name__ == "__main__":
    main()
