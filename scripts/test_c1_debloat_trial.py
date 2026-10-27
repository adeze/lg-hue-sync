import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("c1-debloat-trial.sh").read_text()
FAKE = '''#!/bin/sh
echo "$(basename "$0") $*" >> "$CALLS"
case "$(basename "$0")" in
id) echo 0 ;;
systemd-run) [ "${FAIL_TIMER:-0}" = 0 ] ;;
systemctl)
    case "$*" in
        *rollback*LoadState*) echo not-found ;;
        *LoadState*) echo loaded ;;
        *ActiveState*) if [ -f "$STOPPED" ]; then echo inactive; else echo active; fi ;;
        *UnitFileState*) echo "${UNIT_STATE:-static}" ;;
        *FragmentPath*) echo /lib/systemd/system/remotelogger.service ;;
        'stop remotelogger.service') [ "${FAIL_STOP:-0}" = 0 ] || exit 1; touch "$STOPPED" ;;
    esac ;;
esac
'''


class TrialTests(unittest.TestCase):
    def run_fixture(self, arguments, **environment):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            firmware = root / "firmware"
            firmware.write_text('VERSION="6.5.3"\n')
            script = root / "trial.sh"
            script.write_text(SCRIPT.replace("/etc/webos-release", str(firmware)))
            for name in ("id", "systemctl", "systemd-run"):
                tool = root / name
                tool.write_text(FAKE)
                tool.chmod(0o755)
            calls = root / "calls"
            result = subprocess.run(
                ["sh", str(script), *arguments], capture_output=True, text=True,
                env={**os.environ, "PATH": f"{root}:/usr/bin:/bin", "CALLS": str(calls),
                     "STOPPED": str(root / "stopped"), **environment},
            )
            return result, calls.read_text() if calls.exists() else ""

    def test_default_preview_never_calls_device_tools(self):
        result, calls = self.run_fixture([])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, "")

    def test_changed_state_or_failed_timer_never_masks_or_stops(self):
        for environment in ({"UNIT_STATE": "enabled"}, {"FAIL_TIMER": "1"}):
            with self.subTest(environment=environment):
                result, calls = self.run_fixture(["--trial"], **environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("systemctl mask", calls)
                self.assertNotIn("systemctl stop", calls)

    def test_timer_precedes_mask_and_stop(self):
        result, calls = self.run_fixture(["--trial"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(calls.index("systemd-run"), calls.index("systemctl mask"))
        self.assertLess(calls.index("is-active --quiet"), calls.index("systemctl mask"))
        self.assertLess(calls.index("systemctl mask"), calls.index("systemctl stop"))
        self.assertIn("unmask --runtime remotelogger.service && systemctl start", calls)

    def test_failed_stop_attempts_immediate_restoration(self):
        result, calls = self.run_fixture(["--trial"], FAIL_STOP="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("systemctl unmask --runtime remotelogger.service", calls)
        self.assertIn("systemctl start remotelogger.service", calls)


if __name__ == "__main__":
    unittest.main()
