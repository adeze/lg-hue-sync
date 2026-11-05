#!/usr/bin/env python3
"""Relay QSync's existing NV12 or RGB capture to local HLS and a ten-second lossless clip."""

import argparse
import asyncio
from dataclasses import dataclass
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import math
import re
from pathlib import Path
import shutil
import signal
import ssl
import sys
import threading
import time
from urllib.parse import unquote, urlsplit

MAX_FRAME_BYTES = 320 * 180 * 3
FPS = 6
FRAME_PERIOD = 1 / FPS


class StreamError(Exception):
    pass


@dataclass(frozen=True)
class Frame:
    width: int
    height: int
    captured_ms: int
    pixels: bytes
    pixel_format: str = "rgb24"
    sequence: int | None = None


def header_integer(headers, name, maximum):
    text = headers.get(name, "")
    if not text.isascii() or not text.isdecimal() or len(text) > 20:
        raise StreamError("Invalid frame headers.")
    value = int(text)
    if not 0 < value <= maximum:
        raise StreamError("Invalid frame headers.")
    return value


def validate_frame(headers, body):
    width = header_integer(headers, "x-frame-width", 320)
    height = header_integer(headers, "x-frame-height", 180)
    captured_ms = header_integer(headers, "x-captured-unix-ms", 2**64 - 1)
    length = header_integer(headers, "content-length", MAX_FRAME_BYTES)
    if length != width * height * 3 or len(body) != length:
        raise StreamError("Frame payload size mismatch.")
    if headers.get("content-encoding", "identity").lower() != "identity":
        raise StreamError("Encoded frame payload is unsupported.")
    return Frame(width, height, captured_ms, body)


def nv12_layout(header):
    def integer(name, minimum, maximum):
        value = header.get(name)
        if type(value) is not int or not minimum <= value <= maximum:
            raise StreamError("Invalid NV12 frame header.")
        return value

    if (not isinstance(header, dict) or type(header.get("version")) is not int or header.get("version") != 1
            or header.get("format") != "nv12" or header.get("color_matrix") != "bt601"
            or header.get("color_range") != "limited"):
        raise StreamError("Unsupported NV12 frame format.")
    width, height = integer("width", 2, 640), integer("height", 2, 360)
    sy, suv = integer("stride_y", width, 4096), integer("stride_uv", width, 4096)
    yb, uvb = integer("y_bytes", 1, 2 * 1024 * 1024), integer("uv_bytes", 1, 2 * 1024 * 1024)
    if width % 2 or height % 2 or yb != sy * height or uvb != suv * (height // 2) or yb + uvb > 2 * 1024 * 1024:
        raise StreamError("Invalid NV12 plane dimensions.")
    return width, height, sy, suv, yb, uvb, integer("captured_unix_ms", 1, 2**64 - 1), integer("sequence", 0, 2**64 - 1)


class ChunkedBody:
    def __init__(self, reader):
        self.reader = reader
        self.remaining = 0
        self.terminator = False

    async def readexactly(self, count):
        result = bytearray()
        while len(result) < count:
            if self.remaining == 0:
                if self.terminator and await self.reader.readexactly(2) != b"\r\n":
                    raise StreamError("Invalid HTTP chunk terminator.")
                line = await self.reader.readuntil(b"\r\n")
                token = line[:-2].split(b";", 1)[0]
                if len(line) > 128 or not token or any(c not in b"0123456789abcdefABCDEF" for c in token):
                    raise StreamError("Invalid HTTP chunk length.")
                self.remaining = int(token, 16)
                if not 0 < self.remaining <= 2 * 1024 * 1024 + 4100:
                    raise StreamError("NV12 stream ended or HTTP chunk is oversized.")
                self.terminator = True
            take = min(count - len(result), self.remaining)
            result.extend(await self.reader.readexactly(take))
            self.remaining -= take
        return bytes(result)


async def nv12_frames(address, source_fps=10):
    if type(source_fps) is not int or not 1 <= source_fps <= 15:
        raise StreamError("Source frame rate must be between 1 and 15.")
    host, port, authority, secure = address
    writer = None
    last_sequence = last_timestamp = None
    try:
        async with asyncio.timeout(2):
            reader, writer = await asyncio.open_connection(
                host, port, ssl=ssl.create_default_context() if secure else None, limit=16 * 1024)
            writer.write((f"GET /api/stream/nv12?fps={source_fps} HTTP/1.1\r\n"
                          f"Host: {authority}\r\nConnection: close\r\n"
                          "Accept: application/x-qsync-nv12\r\n\r\n").encode("ascii"))
            await writer.drain()
            block = await reader.readuntil(b"\r\n\r\n")
            lines = block.decode("iso-8859-1").split("\r\n")
            status = int(lines[0].split(" ")[1])
            if status == 409:
                raise StreamError("Screen capture is inactive (409).")
            if status == 429:
                raise StreamError("Another NV12 stream consumer is already connected (429).")
            if status != 200:
                raise StreamError("NV12 source refused the request.")
            headers = {}
            for line in filter(None, lines[1:]):
                key, value = line.split(":", 1)
                key = key.strip().lower()
                if key in headers:
                    raise StreamError("Duplicate stream headers.")
                headers[key] = value.strip().lower()
            if (headers.get("transfer-encoding") != "chunked" or "content-length" in headers
                    or headers.get("content-type", "").split(";", 1)[0] != "application/x-qsync-nv12"
                    or headers.get("content-encoding", "identity") != "identity"):
                raise StreamError("Unsupported NV12 HTTP response.")
        body = ChunkedBody(reader)
        while True:
            async with asyncio.timeout(2):
                length = int.from_bytes(await body.readexactly(4), "big")
                if not 0 < length <= 4096:
                    raise StreamError("Invalid NV12 header length.")
                header = json.loads((await body.readexactly(length)).decode("utf-8"))
                width, height, sy, suv, yb, uvb, captured, sequence = nv12_layout(header)
                if (last_sequence is not None and sequence <= last_sequence
                        or last_timestamp is not None and captured < last_timestamp):
                    raise StreamError("NV12 sequence or capture time moved backwards.")
                planes = await body.readexactly(yb + uvb)
                y = b"".join(planes[row * sy:row * sy + width] for row in range(height))
                uv = b"".join(planes[yb + row * suv:yb + row * suv + width] for row in range(height // 2))
                last_sequence, last_timestamp = sequence, captured
                frame = Frame(width, height, captured, y + uv, "nv12", sequence)
            yield frame
    except StreamError:
        raise
    except (OSError, TimeoutError, ValueError, UnicodeError, asyncio.IncompleteReadError,
            asyncio.LimitOverrunError, TypeError, AttributeError, IndexError):
        raise StreamError("NV12 stream disconnected, stalled or returned invalid data.") from None
    finally:
        if writer is not None:
            writer.close()


class FrameSource:
    def __init__(self):
        self.latest = None
        self.received = 0
        self.fresh = 0

    async def receive(self, address, transport, source_fps=10):
        freshness = Freshness()
        def save(frame):
            now = time.monotonic()
            if frame is not None and not freshness.accept(frame, now):
                self.fresh += 1
            self.received += 1
            self.latest = (frame, now, time.time_ns() // 1_000_000, self.received)
        if transport == "nv12":
            async for frame in nv12_frames(address, source_fps):
                save(frame)
        else:
            while True:
                save(await fetch_frame(address))
                await asyncio.sleep(FRAME_PERIOD)


class Freshness:
    def __init__(self):
        self.last_timestamp = None
        self.last_fresh_at = None

    def accept(self, frame, now):
        self.check(now)
        if self.last_timestamp is not None and frame.captured_ms < self.last_timestamp:
            raise StreamError("Capture timestamp moved backwards.")
        repeated = frame.captured_ms == self.last_timestamp
        if not repeated:
            self.last_timestamp = frame.captured_ms
            self.last_fresh_at = now
        self.check(now)
        return repeated

    def check(self, now):
        if self.last_fresh_at is not None and now - self.last_fresh_at > 2:
            raise StreamError("No fresh captured frames for two seconds.")


def source_address(tv_url):
    try:
        parsed = urlsplit(tv_url)
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
    except ValueError:
        raise StreamError("Invalid TV URL.") from None
    if (parsed.scheme not in {"http", "https"} or not parsed.hostname
            or parsed.username is not None or parsed.password is not None
            or parsed.query or parsed.fragment or parsed.path not in {"", "/"}):
        raise StreamError("TV URL must contain only an HTTP(S) host and optional port.")
    return parsed.hostname, port, parsed.netloc, parsed.scheme == "https"


async def fetch_frame(address):
    host, port, authority, secure = address
    writer = None
    try:
        async with asyncio.timeout(1.5):
            reader, writer = await asyncio.open_connection(
                host, port, ssl=ssl.create_default_context() if secure else None,
                limit=16 * 1024)
            writer.write(("GET /api/stream/frame HTTP/1.1\r\n"
                          f"Host: {authority}\r\nConnection: close\r\n"
                          "Accept: application/octet-stream\r\n\r\n").encode("ascii"))
            await writer.drain()
            block = await reader.readuntil(b"\r\n\r\n")
            lines = block.decode("iso-8859-1").split("\r\n")
            status = int(lines[0].split(" ")[1])
            if status == 202:
                return None
            if status == 409:
                raise StreamError("Screen capture is inactive (409).")
            if status != 200:
                raise StreamError("Frame source refused the request.")
            headers = {}
            for line in lines[1:]:
                if not line:
                    continue
                key, value = line.split(":", 1)
                key = key.lower().strip()
                if key in headers:
                    raise StreamError("Duplicate frame headers.")
                headers[key] = value.strip()
            if "transfer-encoding" in headers:
                raise StreamError("Frame source must provide Content-Length.")
            length = header_integer(headers, "content-length", MAX_FRAME_BYTES)
            # Read exactly one bounded frame; never wait for EOF on a keep-alive peer.
            body = await reader.readexactly(length)
            return validate_frame(headers, body)
    except StreamError:
        raise
    except (OSError, TimeoutError, ValueError, UnicodeError, asyncio.IncompleteReadError,
            asyncio.LimitOverrunError, IndexError):
        raise StreamError("Could not read the frame source.") from None
    finally:
        if writer is not None:
            writer.close()


PLAYER = """<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>QSync screen stream</title><style>body{max-width:960px;margin:2rem auto;padding:1rem;background:#151922;color:#eee;font:16px system-ui}video{width:100%;background:black}a{color:#9fc9ff}body.embedded{margin:0;padding:.5rem;max-width:none}.embedded h1,.embedded .standalone{display:none}.embedded video{max-height:calc(100vh - 5rem);object-fit:contain}</style>
<h1>QSync screen stream</h1><p id="state">Waiting for captured frames…</p>
<video id="video" controls autoplay muted playsinline></video>
<p class="standalone">Existing capture; encoding runs on this host. Lighting settings remain unchanged.</p>
<p class="standalone"><a href="stream.m3u8">HLS playlist</a> · <a href="reference.mkv">First ten seconds, lossless recording</a> · <a href="frames.jsonl">Capture timestamps</a></p>
<script>if(new URLSearchParams(location.search).has("embedded"))document.body.classList.add("embedded");</script>
<script src="hls.min.js"></script>
<script>const video=document.getElementById('video'),state=document.getElementById('state');let started=false,hls=null,playerError='';
function startPlayer(){if(window.Hls&&Hls.isSupported()){hls=new Hls({enableWorker:false,maxBufferLength:12,backBufferLength:6});hls.on(Hls.Events.ERROR,(_event,data)=>{if(data.fatal){playerError='Playback failed: '+data.details;hls.destroy();hls=null;state.textContent=playerError;}});hls.on(Hls.Events.MANIFEST_PARSED,()=>video.play().catch(()=>{}));hls.attachMedia(video);hls.loadSource('stream.m3u8');}else if(video.canPlayType('application/vnd.apple.mpegurl')){video.src='stream.m3u8';video.play().catch(()=>{});}else{playerError='Playback unavailable: this browser supports neither MSE HLS nor native HLS.';}}
video.addEventListener('error',()=>{playerError='Playback failed: video decoding error.';state.textContent=playerError;});window.addEventListener('pagehide',()=>{if(hls)hls.destroy();});
async function poll(){try{const s=await(await fetch('status.json',{cache:'no-store'})).json();if(s.playlist_ready&&!started){started=true;startPlayer();}state.textContent=(playerError?playerError+' ':'')+s.message+(s.transport?` ${s.transport.toUpperCase()} · ${s.encoded_fps} encoded / ${s.fresh_capture_fps} fresh frames/s · ${s.repeated_frames} repeated · last capture arrived ${s.source_age_ms} ms ago`:'');}catch{state.textContent=(playerError?playerError+' ':'')+'Stream server stopped or unavailable.';}}poll();setInterval(poll,500);</script>"""


class LocalServer:
    def __init__(self, directory, status):
        self.directory = directory.resolve()
        self.status = status
        self.server = None
        self.thread = None
        self.request_counts = {"playlist": 0, "segment": 0, "status": 0, "other": 0}
        self.count_lock = threading.Lock()

    async def start(self, port):
        local = self

        class Handler(SimpleHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            extensions_map = {**SimpleHTTPRequestHandler.extensions_map,
                              ".m3u8": "application/vnd.apple.mpegurl", ".ts": "video/mp2t",
                              ".mkv": "video/x-matroska", ".jsonl": "application/x-ndjson"}

            def setup(self):
                super().setup()
                self.connection.settimeout(3)

            def handle(self):
                try:
                    super().handle()
                except OSError:
                    pass

            def log_message(self, *_args):
                pass

            def end_headers(self):
                self.send_header("Cache-Control", "no-store" if urlsplit(self.path).path == "/status.json" else "no-cache")
                super().end_headers()

            def send_header(self, keyword, value):
                if keyword.lower() == "content-length":
                    self.response_length = int(value)
                super().send_header(keyword, value)

            def send_head(self):
                name = unquote(urlsplit(self.path).path).lstrip("/") or "index.html"
                segment = re.fullmatch(r"segment-[0-9]+\.ts", name) is not None
                kind = "playlist" if name == "stream.m3u8" else "segment" if segment else "status" if name == "status.json" else "other"
                with local.count_lock:
                    local.request_counts[kind] += 1
                    counts = local.request_counts.copy()
                allowed = {"index.html", "stream.m3u8", "status.json", "reference.mkv", "frames.jsonl", "hls.min.js", "hls-LICENSE.txt"}
                if (name not in allowed and not segment
                        or not (local.directory / name).resolve().is_relative_to(local.directory)):
                    self.send_error(404)
                    return None
                if name == "status.json":
                    body = json.dumps({**local.status, "playlist_ready": (local.directory / "stream.m3u8").is_file(),
                                       "http_requests": counts}).encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.send_header("Last-Modified", self.date_time_string())
                    self.end_headers()
                    return io.BytesIO(body)
                self.path = "/" + name
                return super().send_head()

            def copyfile(self, source, outputfile):
                # A recording may grow after the standard handler fstats it; honor its advertised length.
                remaining = self.response_length
                while remaining:
                    chunk = source.read(min(64 * 1024, remaining))
                    if not chunk:
                        break
                    outputfile.write(chunk)
                    remaining -= len(chunk)

        class Server(ThreadingHTTPServer):
            daemon_threads = True
            block_on_close = False
            request_queue_size = 16

        self.server = Server(("127.0.0.1", port), lambda *args, **kwargs: Handler(*args, directory=local.directory, **kwargs))
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": .1}, daemon=True)
        self.thread.start()
        return self.server.server_address[1]

    async def close(self):
        if self.server is not None:
            await asyncio.wait_for(asyncio.to_thread(self.server.shutdown), 2)
            self.server.server_close()
            await asyncio.to_thread(self.thread.join, 1)


def encoder_arguments(program, output, frame, fps=FPS):
    return [program, "-hide_banner", "-loglevel", "error", "-nostats", "-nostdin", "-filter_threads", "1",
            "-f", "rawvideo", "-pixel_format", frame.pixel_format, "-video_size", f"{frame.width}x{frame.height}",
            "-framerate", str(fps),
            *(["-colorspace", "smpte170m", "-color_range", "tv"] if frame.pixel_format == "nv12" else []),
            "-i", "pipe:0",
            "-map", "0:v:0", "-an", "-c:v", "libx264", "-threads", "1", "-preset", "ultrafast",
            "-tune", "zerolatency", "-pix_fmt", "yuv420p", "-g", str(fps * 2), "-keyint_min", str(fps * 2), "-sc_threshold", "0",
            "-f", "hls", "-hls_time", "2", "-hls_list_size", "6", "-hls_delete_threshold", "1",
            "-hls_flags", "delete_segments+independent_segments+temp_file",
            "-hls_segment_filename", str(output / "segment-%06d.ts"), str(output / "stream.m3u8"),
            "-map", "0:v:0", "-an", "-c:v", "ffv1", "-threads", "1", "-level", "3", "-pix_fmt", "yuv420p" if frame.pixel_format == "nv12" else "bgr0",
            "-t", "10", "-f", "matroska", str(output / "reference.mkv")]


async def finish_encoder(process):
    if process is None:
        return
    if process.stdin is not None:
        process.stdin.close()
    try:
        await asyncio.wait_for(process.wait(), 5)
    except TimeoutError:
        process.kill()
        await asyncio.wait_for(process.wait(), 2)


async def save_encoder_errors(reader, file):
    written = 0
    while chunk := await reader.read(4096):
        # Continue draining after the log cap so an erroring encoder cannot block on stderr.
        retained = chunk[:max(0, 64 * 1024 - written)]
        file.write(retained)
        written += len(retained)
        file.flush()


async def run(args, program, stop=None):
    address = source_address(args.tv_url)
    fps = getattr(args, "fps", FPS)
    period = 1 / fps
    player_package = Path(__file__).resolve().parents[1] / "frontend/node_modules/hls.js"
    player_script, player_license = player_package / "dist/hls.light.min.js", player_package / "LICENSE"
    if not player_script.is_file() or not player_license.is_file():
        raise StreamError("Local HLS player is missing; run bun install --cwd frontend first.")
    output = Path(args.output).expanduser().resolve()
    try:
        output.mkdir(parents=True, exist_ok=False)
    except FileExistsError:
        raise StreamError("Output directory already exists; choose a new directory.") from None
    (output / "index.html").write_text(PLAYER)
    shutil.copyfile(player_script, output / "hls.min.js")
    shutil.copyfile(player_license, output / "hls-LICENSE.txt")
    status = {"message": "Waiting for captured frames.", "running": True, "frames": 0}
    server = LocalServer(output, status)
    process = None
    log_task = None
    fetch_task = None
    transport = getattr(args, "transport", "nv12")
    source = FrameSource()
    consumed = 0
    repeated_count = 0
    encoder_log = (output / "encoder.log").open("xb")
    stop = stop or asyncio.Event()
    started = time.monotonic()
    freshness = Freshness()
    previous = None
    arrival = None
    arrival_unix_ms = None
    source_status = 202
    next_frame_at = started
    frame_count = 0
    try:
        port = await server.start(args.port)
        print(f"Watch: http://127.0.0.1:{port}/", flush=True)
        print(f"Lossless clip: {output / 'reference.mkv'} (first ten seconds)", flush=True)
        fetch_task = asyncio.create_task(source.receive(address, transport, getattr(args, "source_fps", 10)))
        with (output / "frames.jsonl").open("x") as metadata:
            while not stop.is_set():
                now = time.monotonic()
                if args.duration is not None and now - started >= args.duration:
                    break
                if process is not None and process.returncode is not None:
                    raise StreamError("Host video encoder stopped unexpectedly.")
                frame = previous
                repeated = True
                if fetch_task.done():
                    fetch_task.result()
                    raise StreamError("Frame source stopped unexpectedly.")
                if source.latest is not None and source.latest[3] != consumed:
                    incoming, received_at, received_unix_ms, consumed = source.latest
                    source_status = 200 if incoming is not None else 202
                    if incoming is not None:
                        repeated = freshness.accept(incoming, received_at)
                        frame = incoming
                        arrival, arrival_unix_ms = received_at, received_unix_ms
                freshness.check(time.monotonic())
                if frame is None:
                    if time.monotonic() - started > 10:
                        raise StreamError("No captured frames received within ten seconds.")
                    try:
                        await asyncio.wait_for(stop.wait(), period)
                    except TimeoutError:
                        pass
                    continue
                status["message"] = "Repeating the last capture." if repeated else "Receiving captured frames."
                if previous is not None and (frame.width, frame.height) != (previous.width, previous.height):
                    raise StreamError("Capture dimensions changed; restart the screen stream.")
                if frame.width % 2 or frame.height % 2:
                    raise StreamError("HLS requires even capture dimensions.")
                if process is None:
                    next_frame_at = time.monotonic()
                    process = await asyncio.create_subprocess_exec(
                        *encoder_arguments(program, output, frame, fps), stdin=asyncio.subprocess.PIPE,
                        stdout=asyncio.subprocess.DEVNULL, stderr=asyncio.subprocess.PIPE)
                    log_task = asyncio.create_task(save_encoder_errors(process.stderr, encoder_log))
                encoded_at = time.monotonic()
                process.stdin.write(frame.pixels)
                try:
                    await asyncio.wait_for(process.stdin.drain(), 1)
                except (TimeoutError, OSError):
                    raise StreamError("Host video encoder is not accepting frames.") from None
                if frame_count < fps * 10:
                    metadata.write(json.dumps({"frame": frame_count, "captured_unix_ms": frame.captured_ms,
                        "arrival_unix_ms": arrival_unix_ms, "arrival_elapsed_ms": round((arrival - started) * 1000, 3),
                        "encoded_elapsed_ms": round((encoded_at - started) * 1000, 3),
                        "repeated": repeated, "source_status": source_status,
                        "width": frame.width, "height": frame.height, "pixel_format": frame.pixel_format,
                        "sequence": frame.sequence}) + "\n")
                    metadata.flush()
                frame_count += 1
                repeated_count += int(repeated)
                status.update(frames=frame_count, captured_unix_ms=frame.captured_ms, repeated=repeated,
                              transport=transport, encoded_fps=fps, source_frames=source.fresh,
                              fresh_capture_fps=round(source.fresh / max(time.monotonic() - started, .001), 2),
                              repeated_frames=repeated_count,
                              source_age_ms=round((time.monotonic() - freshness.last_fresh_at) * 1000))
                previous = frame
                next_frame_at = max(next_frame_at + period, time.monotonic())
                try:
                    await asyncio.wait_for(stop.wait(), max(0.001, next_frame_at - time.monotonic()))
                except TimeoutError:
                    pass
        if frame_count == 0:
            raise StreamError("No frames captured before the stream stopped.")
    finally:
        status.update(running=False, message="Screen stream stopped.")
        try:
            if fetch_task is not None:
                fetch_task.cancel()
                await asyncio.gather(fetch_task, return_exceptions=True)
            await finish_encoder(process)
            if log_task is not None:
                await asyncio.wait_for(log_task, 1)
        finally:
            if log_task is not None and not log_task.done():
                log_task.cancel()
                await asyncio.gather(log_task, return_exceptions=True)
            encoder_log.close()
            (output / "status.json").write_text(json.dumps(status) + "\n")
            await server.close()
    if process is not None and process.returncode != 0:
        raise StreamError("Host video encoder did not finish successfully.")
    return frame_count


def positive_duration(text):
    try:
        value = float(text)
    except ValueError:
        raise argparse.ArgumentTypeError("duration must be positive and finite") from None
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError("duration must be positive and finite")
    return value


async def main_async(args, program):
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for name in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(name, stop.set)
    return await run(args, program, stop)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tv-url", required=True, help="QSync HTTP(S) origin; no credentials or path")
    parser.add_argument("--output", required=True, type=Path, help="new output directory; existing directories are refused")
    parser.add_argument("--port", default=18090, type=int, help="localhost player port (default: 18090)")
    parser.add_argument("--fps", type=int, choices=range(1, 31), default=10, help="encoded frame rate, 1–30 fps (default: 10)")
    parser.add_argument("--source-fps", type=int, choices=range(1, 16), default=10, help="NV12 export ceiling, 1–15 fps (default: 10)")
    parser.add_argument("--transport", choices=["nv12", "rgb"], default="nv12", help="persistent NV12 feed (default) or legacy RGB polling")
    parser.add_argument("--duration", type=positive_duration, help="stop encoder and HTTP server after this many seconds")
    args = parser.parse_args()
    if not 0 <= args.port <= 65535:
        parser.error("port must be between 0 and 65535")
    program = shutil.which("ffmpeg")
    if program is None:
        parser.error("ffmpeg is required on this host")
    try:
        asyncio.run(main_async(args, program))
    except (StreamError, OSError, TimeoutError):
        print("Screen stream stopped: " + (str(sys.exception()) if isinstance(sys.exception(), StreamError) else "local I/O or encoder cleanup failed."), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
