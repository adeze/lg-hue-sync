"""Local-only fixtures; no TV, settings or lighting output access."""

import argparse
import asyncio
import contextlib
import io
import http.client
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

import hls_screen as relay


class ValidationTests(unittest.TestCase):
    def test_frames_and_relative_freshness_are_bounded(self):
        headers = {"x-frame-width": "2", "x-frame-height": "2",
                   "x-captured-unix-ms": "1000", "content-length": "12"}
        frame = relay.validate_frame(headers, bytes(12))
        for key, value in [("x-frame-width", "321"), ("x-frame-height", "181"),
                           ("x-frame-width", "0"), ("x-frame-width", "nan"),
                           ("x-captured-unix-ms", "-1"), ("content-length", "9999999"),
                           ("content-length", "13"), ("content-encoding", "gzip")]:
            with self.subTest(key=key, value=value), self.assertRaises(relay.StreamError):
                relay.validate_frame({**headers, key: value}, bytes(12))
        for body in [bytes(11), bytes(13), bytes(relay.MAX_FRAME_BYTES + 1)]:
            with self.assertRaises(relay.StreamError):
                relay.validate_frame(headers, body)
        freshness = relay.Freshness()
        self.assertFalse(freshness.accept(frame, 10))
        self.assertTrue(freshness.accept(frame, 11))
        with self.assertRaises(relay.StreamError):
            freshness.accept(frame, 12.001)
        with self.assertRaises(relay.StreamError):
            freshness.accept(relay.Frame(2, 2, 999, bytes(12)), 11)
        self.assertFalse(freshness.accept(relay.Frame(2, 2, 1167, bytes(12)), 11.5))
        for url in ["file:///private", "http://user:secret@localhost", "http://localhost?secret=x"]:
            with self.assertRaises(relay.StreamError) as error:
                relay.source_address(url)
            self.assertNotIn("secret", str(error.exception))


        header = {"version": 1, "format": "nv12", "width": 4, "height": 2,
                  "stride_y": 6, "stride_uv": 6, "y_bytes": 12, "uv_bytes": 6,
                  "captured_unix_ms": 1000, "sequence": 1,
                  "color_matrix": "bt601", "color_range": "limited"}
        self.assertEqual(relay.nv12_layout(header)[:6], (4, 2, 6, 6, 12, 6))
        for key, value in [("width", 641), ("height", 3), ("stride_y", 4097),
                           ("stride_uv", 3), ("y_bytes", 1), ("uv_bytes", 2**30),
                           ("sequence", -1), ("sequence", True), ("captured_unix_ms", 0),
                           ("color_matrix", "bt709"), ("format", "rgb")]:
            with self.subTest(key=key), self.assertRaises(relay.StreamError):
                relay.nv12_layout({**header, key: value})
        for value in [[], None, "bad"]:
            with self.assertRaises(relay.StreamError):
                relay.nv12_layout(value)


class RelayTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.servers = []
        self.program = shutil.which("ffmpeg")

    async def asyncTearDown(self):
        for server in self.servers:
            server.close()
            await server.wait_closed()

    async def source(self, mode="frames", delay=0, initial_waits=2):
        counter = 0
        payloads = []

        async def handle(reader, writer):
            nonlocal counter
            try:
                request = await reader.readuntil(b"\r\n\r\n")
                self.assertTrue(request.startswith(b"GET /api/stream/frame HTTP/1.1\r\n"))
                await asyncio.sleep(delay)
                counter += 1
                if mode == "inactive":
                    writer.write(b"HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\n\r\n")
                elif counter <= initial_waits:
                    writer.write(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                else:
                    # Repeated timestamp/pixels every second request exercises explicit repeat receipts.
                    index = (counter - initial_waits - 1) // 2
                    color = bytes([30 + index % 100, 90, 170 - index % 100])
                    body = color * (64 * 36)
                    payloads.append(body)
                    writer.write(("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n"
                                  f"X-Frame-Width: 64\r\nX-Frame-Height: 36\r\nX-Captured-Unix-Ms: {1000 + index * 334}\r\n"
                                  f"Content-Length: {len(body)}\r\n\r\n").encode() + body)
                await writer.drain()
            except (ConnectionError, asyncio.IncompleteReadError):
                pass
            finally:
                writer.close()

        server = await asyncio.start_server(handle, "127.0.0.1", 0)
        self.servers.append(server)
        return f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}", payloads

    @unittest.skipUnless(shutil.which("ffmpeg"), "host ffmpeg required")
    async def test_local_source_produces_bounded_hls_and_exact_lossless_reference(self):
        url, payloads = await self.source()
        output = Path(self.directory.name) / "capture"
        args = argparse.Namespace(transport="rgb", tv_url=url, output=output, port=0, duration=11.2)
        with contextlib.redirect_stdout(io.StringIO()) as console:
            count = await relay.run(args, self.program)
        self.assertGreater(count, 60)
        self.assertNotIn(url, console.getvalue())
        metadata = [json.loads(line) for line in (output / "frames.jsonl").read_text().splitlines()]
        self.assertEqual(len(metadata), 60)
        self.assertTrue(any(row["repeated"] for row in metadata))
        self.assertTrue(all(row["source_status"] == 200 for row in metadata))
        playlist = (output / "stream.m3u8").read_text()
        self.assertIn("#EXT-X-ENDLIST", playlist)
        self.assertLessEqual(playlist.count("#EXTINF:"), 6)
        self.assertLessEqual(len(list(output.glob("segment-*.ts"))), 7)
        decoded = subprocess.run([self.program, "-v", "error", "-i", str(output / "reference.mkv"),
                                  "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"],
                                 capture_output=True, check=True, timeout=10).stdout
        expected = [bytes([30 + ((row["captured_unix_ms"] - 1000) // 334) % 100,
                           90, 170 - ((row["captured_unix_ms"] - 1000) // 334) % 100]) * (64 * 36)
                    for row in metadata]
        self.assertEqual(decoded, b"".join(expected))
        self.assertFalse(json.loads((output / "status.json").read_text())["running"])
        server = relay.LocalServer(output, {"message": "fixture", "running": True})
        port = await server.start(0)
        def check_http():
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
            try:
                connection.request("GET", "/stream.m3u8")
                response = connection.getresponse()
                body = response.read()
                self.assertEqual(response.status, 200)
                self.assertEqual(int(response.getheader("Content-Length")), len(body))
                self.assertIsNotNone(response.getheader("Date"))
                modified = response.getheader("Last-Modified")
                self.assertIsNotNone(modified)
                self.assertEqual(response.getheader("Cache-Control"), "no-cache")
                socket = connection.sock
                connection.request("HEAD", "/stream.m3u8")
                response = connection.getresponse()
                self.assertEqual(response.read(), b"")
                self.assertEqual(response.status, 200)
                self.assertEqual(int(response.getheader("Content-Length")), len(body))
                self.assertEqual(response.getheader("Last-Modified"), modified)
                self.assertIsNotNone(response.getheader("Date"))
                self.assertIs(connection.sock, socket)
                connection.request("GET", "/stream.m3u8", headers={"If-Modified-Since": modified})
                response = connection.getresponse()
                self.assertEqual(response.status, 304)
                self.assertEqual(response.read(), b"")
                connection.request("GET", "/status.json")
                response = connection.getresponse()
                body = response.read()
                self.assertEqual(int(response.getheader("Content-Length")), len(body))
                self.assertEqual(response.getheader("Cache-Control"), "no-store")
                counts = json.loads(body)["http_requests"]
                self.assertEqual(counts["playlist"], 3)
                self.assertEqual(counts["status"], 1)
                for asset in ["hls.min.js", "hls-LICENSE.txt"]:
                    connection.request("GET", "/" + asset)
                    response = connection.getresponse()
                    body = response.read()
                    self.assertEqual(response.status, 200)
                    self.assertEqual(int(response.getheader("Content-Length")), len(body))
                    self.assertGreater(len(body), 100)
                    self.assertIsNone(response.getheader("Access-Control-Allow-Origin"))
                connection.request("GET", "/?embedded=1")
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertIn(b"hls.min.js", response.read())
                for path in ["/../encoder.log", "/encoder.log", "/folder/"]:
                    connection.request("GET", path)
                    response = connection.getresponse()
                    response.read()
                    self.assertEqual(response.status, 404)
            finally:
                connection.close()
        try:
            await asyncio.to_thread(check_http)
        finally:
            await server.close()

    @unittest.skipUnless(shutil.which("ffmpeg"), "host ffmpeg required")
    async def test_slow_source_keeps_wall_clock_rate_with_explicit_repeats(self):
        url, _ = await self.source(delay=.35, initial_waits=0)
        output = Path(self.directory.name) / "slow"
        args = argparse.Namespace(transport="rgb", tv_url=url, output=output, port=0, duration=2.6)
        with contextlib.redirect_stdout(io.StringIO()):
            count = await relay.run(args, self.program)
        metadata = [json.loads(line) for line in (output / "frames.jsonl").read_text().splitlines()]
        self.assertGreaterEqual(count, 12)
        repeats = [row for row in metadata if row["repeated"]]
        self.assertGreater(len(repeats), count // 2)
        self.assertTrue(any(a["arrival_elapsed_ms"] == b["arrival_elapsed_ms"]
                            and a["encoded_elapsed_ms"] < b["encoded_elapsed_ms"]
                            for a, b in zip(metadata, metadata[1:])))
        decoded = subprocess.run([self.program, "-v", "error", "-i", str(output / "reference.mkv"),
                                  "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"],
                                 capture_output=True, check=True, timeout=10).stdout
        clip_seconds = len(decoded) / (64 * 36 * 3 * relay.FPS)
        encoded_wall_seconds = (metadata[-1]["encoded_elapsed_ms"] - metadata[0]["encoded_elapsed_ms"]) / 1000 + relay.FRAME_PERIOD
        self.assertAlmostEqual(clip_seconds, encoded_wall_seconds, delta=.2)


    async def nv12_source(self, mode="frames", interval=.1):
        connections = []
        def packet(index):
            header = {"version": 1, "format": "nv12", "width": 64, "height": 36,
                      "stride_y": 66, "stride_uv": 66, "y_bytes": 66 * 36, "uv_bytes": 66 * 18,
                      "captured_unix_ms": 1000 + round(index * interval * 1000), "sequence": index,
                      "color_matrix": "bt601", "color_range": "limited"}
            if mode == "bad_size":
                header["y_bytes"] = 2**30
            if mode == "backwards":
                header["sequence"] = 0
            body = bytes([16 + index % 200]) * (66 * 36) + bytes([90, 160]) * (33 * 18)
            encoded = json.dumps(header).encode()
            return len(encoded).to_bytes(4, "big") + encoded + body

        async def handle(reader, writer):
            task = asyncio.current_task()
            connections.append(task)
            try:
                request = await reader.readuntil(b"\r\n\r\n")
                self.assertTrue(request.startswith(b"GET /api/stream/nv12?fps=10 HTTP/1.1\r\n"))
                writer.write(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/x-qsync-nv12\r\n\r\n")
                if mode == "stale":
                    await asyncio.sleep(3)
                    return
                count = 2 if mode in {"backwards", "ended"} else 1000
                for index in range(count):
                    data = packet(index)
                    if mode == "bad_header":
                        data = (4097).to_bytes(4, "big")
                    if mode == "bad_chunk":
                        writer.write(b"ffffff\r\n")
                        await writer.drain()
                        await asyncio.sleep(3)
                        return
                    for start in range(0, len(data), 113):
                        chunk = data[start:start + 113]
                        wire = f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n"
                        # Deliberately split HTTP syntax and packet headers across socket writes.
                        writer.write(wire[:1]); await writer.drain()
                        writer.write(wire[1:]); await writer.drain()
                    if mode in {"bad_size", "bad_header"}:
                        await asyncio.sleep(3)
                        return
                    await asyncio.sleep(interval)
                writer.write(b"0\r\n\r\n")
                await writer.drain()
            except (OSError, asyncio.IncompleteReadError):
                pass
            finally:
                writer.close()
        server = await asyncio.start_server(handle, "127.0.0.1", 0)
        self.servers.append(server)
        async def close_connections():
            for task in connections:
                task.cancel()
            await asyncio.gather(*connections, return_exceptions=True)
        self.addAsyncCleanup(close_connections)
        return f"http://127.0.0.1:{server.sockets[0].getsockname()[1]}"

    async def test_nv12_fragmented_frames_bounds_stale_and_disconnect(self):
        url = await self.nv12_source()
        stream = relay.nv12_frames(relay.source_address(url))
        try:
            frame = await anext(stream)
            self.assertEqual((frame.width, frame.height, frame.pixel_format, frame.sequence), (64, 36, "nv12", 0))
            self.assertEqual(frame.pixels, bytes([16]) * (64 * 36) + bytes([90, 160]) * (32 * 18))
            self.assertEqual((await anext(stream)).sequence, 1)
        finally:
            await stream.aclose()
        for mode in ["bad_size", "bad_header", "bad_chunk", "backwards", "ended", "stale"]:
            with self.subTest(mode=mode):
                url = await self.nv12_source(mode)
                stream = relay.nv12_frames(relay.source_address(url))
                try:
                    with self.assertRaises(relay.StreamError):
                        async for _ in stream:
                            pass
                finally:
                    await stream.aclose()

    @unittest.skipUnless(shutil.which("ffmpeg"), "host ffmpeg required")
    async def test_nv12_hls_and_lossless_yuv_reference(self):
        url = await self.nv12_source(interval=1/15)
        output = Path(self.directory.name) / "nv12"
        args = argparse.Namespace(transport="nv12", fps=10, tv_url=url, output=output, port=0, duration=2.6)
        with contextlib.redirect_stdout(io.StringIO()):
            count = await relay.run(args, self.program)
        self.assertGreaterEqual(count, 12)
        metadata = [json.loads(line) for line in (output / "frames.jsonl").read_text().splitlines()]
        decoded = subprocess.run([self.program, "-v", "error", "-i", str(output / "reference.mkv"),
                                  "-f", "rawvideo", "-pix_fmt", "nv12", "pipe:1"],
                                 capture_output=True, check=True, timeout=10).stdout
        expected = b"".join(bytes([16 + row["sequence"] % 200]) * (64 * 36)
                            + bytes([90, 160]) * (32 * 18) for row in metadata)
        self.assertEqual(decoded, expected)
        self.assertIn("#EXTM3U", (output / "stream.m3u8").read_text())
        status = json.loads((output / "status.json").read_text())
        self.assertEqual(status["transport"], "nv12")
        self.assertGreater(status["source_frames"], status["frames"])

    async def test_inactive_encoder_failure_and_existing_directory_stop_cleanly(self):
        inactive, _ = await self.source("inactive")
        output = Path(self.directory.name) / "inactive"
        args = argparse.Namespace(transport="rgb", tv_url=inactive, output=output, port=0, duration=1)
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(relay.StreamError) as error:
            await relay.run(args, shutil.which("false"))
        self.assertIn("inactive", str(error.exception))
        self.assertNotIn(inactive, str(error.exception))
        self.assertFalse((output / "reference.mkv").exists())
        preserved = output / "preserved.txt"
        preserved.write_text("keep")
        with self.assertRaises(relay.StreamError):
            await relay.run(args, shutil.which("false"))
        self.assertEqual(preserved.read_text(), "keep")
        active, _ = await self.source()
        args = argparse.Namespace(transport="rgb", tv_url=active, output=Path(self.directory.name) / "failed", port=0, duration=3)
        before = time.monotonic()
        with contextlib.redirect_stdout(io.StringIO()), self.assertRaises((relay.StreamError, BrokenPipeError)):
            await relay.run(args, shutil.which("false"))
        self.assertLess(time.monotonic() - before, 3)


if __name__ == "__main__":
    unittest.main()
