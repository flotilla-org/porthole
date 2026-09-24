import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_candidate as candidate


class CandidateTests(unittest.TestCase):
    def metadata(self):
        revision = "a" * 40
        return {"packages": [
            {"name": name, "source": f"git+https://github.com/flotilla-org/jackstay?rev={revision}#{revision}",
             "manifest_path": f"/checkout/crates/{name}/Cargo.toml"}
            for name in ("jackstay", "jackstay-graph")
        ]}

    def test_resolves_shared_exact_pin(self):
        self.assertEqual(candidate.resolved_jackstay(self.metadata()), ("a" * 40, Path("/checkout")))

    def test_rejects_drift_and_noncanonical_sources(self):
        original = self.metadata()
        for replacement in (None, "path+file:///tmp/jackstay", original["packages"][0]["source"].replace("#aaa", "#bbb"),
                            original["packages"][0]["source"].replace("flotilla-org", "other")):
            metadata = copy.deepcopy(original)
            for package in metadata["packages"]:
                package["source"] = replacement
            with self.subTest(source=replacement), self.assertRaises(ValueError):
                candidate.resolved_jackstay(metadata)
        metadata = copy.deepcopy(original)
        metadata["packages"][1]["source"] = None
        with self.assertRaises(ValueError):
            candidate.resolved_jackstay(metadata)

    def test_compatibility_reads_abi_from_jackstay_and_wire_from_porthole(self):
        with tempfile.TemporaryDirectory() as temporary:
            jackstay = Path(temporary) / "jackstay"
            porthole = Path(temporary) / "porthole"
            header = jackstay / "crates/jackstay/include/capture_transfer.h"
            wire = porthole / "crates/jackstay-bridge/src/wire.rs"
            header.parent.mkdir(parents=True)
            wire.parent.mkdir(parents=True)
            header.write_text("#define FT_ABI_VERSION_MAJOR 2\n#define FT_ABI_VERSION_MINOR 5\n")
            wire.write_text("pub const VERSION: u8 = 7;\n")
            self.assertEqual(candidate.compatibility(jackstay, porthole), {
                "jackstay_c_abi": {"major": 2, "minor": 5}, "jackstay_bridge_wire": 7,
            })
            wire.write_text("// no version\n")
            with self.assertRaises(ValueError):
                candidate.compatibility(jackstay, porthole)

    def app(self, root):
        app = root / "Porthole.app"
        for relative in candidate.RESOURCES + tuple(f"Contents/MacOS/{name}" for name in candidate.BINARIES):
            path = app / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"fixture")
            path.chmod(0o755 if "/MacOS/" in relative else 0o644)
        return app

    def test_manifest_covers_every_payload_and_requires_bridge(self):
        with tempfile.TemporaryDirectory() as temporary:
            app = self.app(Path(temporary))
            files = candidate.inventory(app)
            self.assertEqual(len(files), 7)
            self.assertEqual(files["Porthole.app/Contents/MacOS/portholed"]["sha256"], candidate.digest(app / "Contents/MacOS/portholed"))
            (app / "Contents/MacOS/jackstay-bridge").unlink()
            with self.assertRaisesRegex(ValueError, "missing"):
                candidate.inventory(app)

    def test_rejects_extra_payload_and_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            app = self.app(Path(temporary))
            extra = app / "Contents/MacOS/extra"
            extra.write_bytes(b"extra")
            with self.assertRaisesRegex(ValueError, "unexpected"):
                candidate.inventory(app)
            extra.unlink()
            bridge = app / "Contents/MacOS/jackstay-bridge"
            bridge.unlink()
            bridge.symlink_to("portholed")
            with self.assertRaisesRegex(ValueError, "symlink"):
                candidate.inventory(app)

    def test_linkage_rejects_host_libraries_and_build_rpaths(self):
        for dependency, rpath, valid in (
            ("/usr/lib/libSystem.B.dylib", "", True),
            ("/opt/homebrew/lib/libjackstay.dylib", "", False),
            ("@rpath/libjackstay.dylib", "", False),
            ("/usr/lib/libSystem.B.dylib", "/tmp/old-checkout/target", False),
        ):
            def output(*args):
                if "lipo" in args[0]:
                    return "arm64"
                if "-L" in args:
                    return f"binary:\n\t{dependency} (compatibility version 1.0.0)"
                return f"cmd LC_RPATH\n cmdsize 80\n path {rpath} (offset 12)" if rpath else ""
            with self.subTest(dependency=dependency, rpath=rpath), patch.object(candidate, "output", output):
                if valid:
                    candidate.verify_linkage(Path("/fixture"))
                else:
                    with self.assertRaises(ValueError):
                        candidate.verify_linkage(Path("/fixture"))

    def test_removes_only_active_swift_toolchain_fallback(self):
        toolchain = "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr"
        paths = [f"{toolchain}/lib/swift-6.2/macosx", "/tmp/other-build/lib", "/usr/lib/swift"]
        commands = "\n".join(f"cmd LC_RPATH\n cmdsize 80\n path {p} (offset 12)" for p in paths)
        with patch.object(candidate, "output", side_effect=[f"{toolchain}/bin/swiftc", commands]), patch.object(candidate, "run") as run:
            candidate.remove_swift_toolchain_rpaths(Path("/fixture"))
        run.assert_called_once_with("/usr/bin/install_name_tool", "-delete_rpath", paths[0], "/fixture/Contents/MacOS/PortholeHelper")


if __name__ == "__main__":
    unittest.main()
