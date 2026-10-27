import contextlib
import io
import subprocess
import unittest
from unittest.mock import patch

import audit_tv_debloat as audit


class AuditTests(unittest.TestCase):
    def test_invalid_destination_never_runs_ssh(self):
        for args in (["-oProxyCommand=bad"], ["root@example"], ["example", "--port", "0"]):
            with self.subTest(args=args), patch.object(audit.subprocess, "run") as run:
                with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                    audit.main(args)
                run.assert_not_called()

    def test_transport_streams_read_only_payload_and_propagates_failure(self):
        with patch.object(audit.subprocess, "run", return_value=subprocess.CompletedProcess([], 255)) as run:
            self.assertEqual(audit.main(["tv", "--port", "9922"]), 255)
        command = run.call_args.args[0]
        self.assertIn("StrictHostKeyChecking=yes", command)
        self.assertEqual(command[-2:], ["root@tv", "sh -s"])
        self.assertEqual(run.call_args.kwargs["input"], audit.AUDIT)
        self.assertEqual(run.call_args.kwargs["timeout"], 60)
        self.assertNotRegex(audit.AUDIT, r"\b(stop|mask|pkill|mount|setSystemSettings|reboot)\b")

    def test_timeout_is_explicit(self):
        with patch.object(audit.subprocess, "run", side_effect=subprocess.TimeoutExpired("ssh", 60)):
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(audit.main(["tv"]), 124)

    def test_payload_is_posix_shell_and_covers_dependency_and_luna_activation(self):
        subprocess.run(["sh", "-n"], input=audit.AUDIT, text=True, check=True)
        for field in ("-p RequiredBy", "-p BoundBy", "-p Restart", "services.d", "amazon-alexa-adapter"):
            self.assertIn(field, audit.AUDIT)


if __name__ == "__main__":
    unittest.main()
