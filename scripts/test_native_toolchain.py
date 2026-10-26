import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("webos-native-build.sh")
HEADER = "Class: ELF32\nData: 2's complement, little endian\nMachine: ARM\nFlags: 0x5000200, Version5 EABI, soft-float ABI\n"
VERSIONS = "Name: GLIBC_2.4\nName: GLIBC_2.28\n"
FAKE_RUSTUP = r'''
import json, os, pathlib, sys
with open(os.environ["TEST_CALLS"], "a") as calls:
    calls.write(json.dumps({"argv":sys.argv[1:],"env":{key:os.environ.get(key) for key in (
        "RUSTC","CARGO_TARGET_DIR","CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER",
        "CC_armv7_unknown_linux_gnueabi","AR_armv7_unknown_linux_gnueabi","RANLIB_armv7_unknown_linux_gnueabi")}})+"\n")
if sys.argv[1:] == ["which","--toolchain","stable","rustc"]:
    if os.environ.get("TEST_WHICH_FAIL"):
        sys.exit(38)
    print(os.environ["TEST_RUSTC"])
elif sys.argv[1:5] == ["run","stable","cargo","build"]:
    name = sys.argv[sys.argv.index("--bin")+1]
    binary = pathlib.Path(os.environ["CARGO_TARGET_DIR"])/"armv7-unknown-linux-gnueabi"/"release"/name
    binary.parent.mkdir(parents=True,exist_ok=True)
    binary.write_bytes(b"new build fixture")
    sys.exit(37 if os.environ.get("TEST_BUILD_FAIL") else 0)
else:
    sys.exit(99)
'''
FAKE_READELF = r'''
import os, pathlib, sys
if not pathlib.Path(sys.argv[-1]).is_file():
    sys.exit(97)
if sys.argv[1] == "-h":
    print(os.environ["TEST_HEADER"])
elif sys.argv[1] == "--version-info":
    print(os.environ["TEST_VERSIONS"])
elif sys.argv[1] == "-l":
    print(os.environ["TEST_PROGRAM_HEADERS"])
else:
    sys.exit(98)
'''


class NativeToolchainTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.bin = self.root / "installed tools" / "bin"
        self.sdk = self.root / "native SDK"
        self.bin.mkdir(parents=True)
        (self.sdk / "bin").mkdir(parents=True)
        self.script = self.bin / "webos-native-build"
        shutil.copyfile(SCRIPT, self.script)
        (self.bin / "webos-native-cc").write_text('#!/bin/sh\n[ "$1" = --version ] || exit 99\n')
        (self.bin / "webos-native-cc").chmod(0o755)
        self.tool(self.bin / "rustup", FAKE_RUSTUP)
        self.tool(self.sdk / "bin/arm-webos-linux-gnueabi-readelf", FAKE_READELF)
        self.manifest = self.root / "project space" / "Cargo.toml"
        self.manifest.parent.mkdir()
        self.manifest.write_text("[package]\nname = 'fixture-daemon'\nversion = '0.1.0'\n")
        self.output = self.root / "release space" / "fixture-daemon"
        self.output.parent.mkdir()
        self.output.write_bytes(b"previous deployable")
        self.calls = self.root / "calls.jsonl"
        self.env = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
            "WEBOS_SDK": str(self.sdk),
            "TEST_RUSTC": str(self.root / "stable toolchain/bin/rustc"),
            "TEST_CALLS": str(self.calls),
            "TEST_HEADER": HEADER,
            "TEST_VERSIONS": VERSIONS,
            "TEST_PROGRAM_HEADERS": "[Requesting program interpreter: /lib/ld-linux.so.3]",
            "RUSTC": "/wrong/shadowed/rustc",
            "AR_armv7_unknown_linux_gnueabi": "/wrong/ar",
            "RANLIB_armv7_unknown_linux_gnueabi": "/wrong/ranlib",
        }
        self.env.pop("TEST_BUILD_FAIL", None)
        self.env.pop("TEST_WHICH_FAIL", None)
        self.env.pop("WEBOS_RUSTC", None)
        self.env.pop("WEBOS_CARGO", None)

    def tool(self, path, source):
        path.write_text(f"#!{sys.executable}\n" + source)
        path.chmod(0o755)

    def build(self, **changes):
        return subprocess.run(
            ["bash", str(self.script), str(self.manifest), "fixture-daemon", str(self.output)],
            env={**self.env, **changes}, capture_output=True, text=True, timeout=10,
        )

    def test_valid_soft_float_binary_replaces_output_using_rustup_and_sdk_tools(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_bytes(), b"new build fixture")
        self.assertEqual(list(self.output.parent.glob(self.output.name + ".*")), [])
        self.assertIn("Verified ELF32 ARM", result.stdout)
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual(calls[0]["argv"], ["which", "--toolchain", "stable", "rustc"])
        self.assertEqual(calls[1]["argv"], ["run", "stable", "cargo", "build", "--locked", "--release", "--target", "armv7-unknown-linux-gnueabi", "--manifest-path", str(self.manifest), "--bin", "fixture-daemon"])
        env = calls[1]["env"]
        self.assertEqual(env["RUSTC"], self.env["TEST_RUSTC"])
        self.assertEqual(env["CARGO_TARGET_DIR"], str(self.manifest.parent / "target/native"))
        for key in ("CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER", "CC_armv7_unknown_linux_gnueabi"):
            self.assertEqual(env[key], str(self.bin / "webos-native-cc"))
        self.assertEqual(env["AR_armv7_unknown_linux_gnueabi"], str(self.sdk / "bin/arm-webos-linux-gnueabi-ar"))
        self.assertEqual(env["RANLIB_armv7_unknown_linux_gnueabi"], str(self.sdk / "bin/arm-webos-linux-gnueabi-ranlib"))

    def test_rejected_artifacts_and_failed_builds_preserve_previous_output(self):
        cases = {
            "hard float": {"TEST_HEADER": HEADER.replace("soft-float", "hard-float")},
            "ELF64": {"TEST_HEADER": HEADER.replace("ELF32", "ELF64")},
            "non ARM": {"TEST_HEADER": HEADER.replace("Machine: ARM", "Machine: Advanced Micro Devices X86-64")},
            "big endian": {"TEST_HEADER": HEADER.replace("little endian", "big endian")},
            "wrong loader": {"TEST_PROGRAM_HEADERS": "[Requesting program interpreter: /lib/ld-linux-armhf.so.3]"},
            "newer glibc": {"TEST_VERSIONS": "Name: GLIBC_2.29\n"},
            "newer glibc patch": {"TEST_VERSIONS": "Name: GLIBC_2.28.1\n"},
            "missing versions": {"TEST_VERSIONS": "No version information found in this file."},
            "compiler failure": {"TEST_BUILD_FAIL": "1"},
            "rustc selection failure": {"TEST_WHICH_FAIL": "1"},
        }
        for name, changes in cases.items():
            with self.subTest(name=name):
                result = self.build(**changes)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(self.output.read_bytes(), b"previous deployable")
                self.assertEqual(list(self.output.parent.glob(self.output.name + ".*")), [])
                if "TEST_WHICH_FAIL" in changes:
                    last_call = json.loads(self.calls.read_text().splitlines()[-1])
                    self.assertEqual(last_call["argv"], ["which", "--toolchain", "stable", "rustc"])

    def test_supplied_toolchain_bypasses_rustup_and_rejects_incomplete_setup(self):
        compiler = self.root / "Nix toolchain/bin/rustc"
        cargo = compiler.with_name("cargo")
        compiler.parent.mkdir(parents=True)
        target = self.root / "target libraries"
        target.mkdir()
        self.tool(compiler, 'import os; print(os.environ["TEST_TARGET_LIBDIR"])\n')
        self.tool(cargo, FAKE_RUSTUP.replace(
            'sys.argv[1:5] == ["run","stable","cargo","build"]',
            'sys.argv[1:2] == ["build"]',
        ))
        supplied = {
            "WEBOS_RUSTC": str(compiler), "WEBOS_CARGO": str(cargo),
            "TEST_WHICH_FAIL": "1", "TEST_TARGET_LIBDIR": str(target),
        }
        result = self.build(**supplied)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["argv"][0], "build")
        self.assertEqual(calls[0]["env"]["RUSTC"], str(compiler))

        for changes in [
            {"WEBOS_RUSTC": ""}, {"WEBOS_CARGO": ""},
            {"WEBOS_RUSTC": str(self.root / "missing-rustc")},
            {"WEBOS_CARGO": str(self.root / "missing-cargo")},
            {"TEST_BUILD_FAIL": "1"},
        ]:
            with self.subTest(changes=changes):
                self.output.write_bytes(b"previous deployable")
                result = self.build(**{**supplied, **changes})
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.output.read_bytes(), b"previous deployable")

        # An explicit compiler must already contain the target; setup must not
        # mutate rustup or try to bootstrap a sysroot when that evidence is absent.
        (self.sdk / "bin/arm-webos-linux-gnueabi-gcc").write_text('#!/bin/sh\nexit 0\n')
        (self.sdk / "bin/arm-webos-linux-gnueabi-gcc").chmod(0o755)
        result = subprocess.run(
            ["bash", str(SCRIPT.with_name("setup-native-toolchain.sh"))],
            env={**self.env, **supplied, "TEST_TARGET_LIBDIR": str(target / "missing"),
                 "WEBOS_NATIVE_HOME": str(self.root / "not-created")},
            capture_output=True, text=True, timeout=10,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no ARMv7 target libraries", result.stderr)
        self.assertFalse((self.root / "not-created").exists())


if __name__ == "__main__":
    unittest.main()
