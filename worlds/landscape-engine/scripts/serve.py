#!/usr/bin/env python3
"""Serves dist/ for local testing, with what `python -m http.server` lacks: HTTP Range requests,
which the viewer streams scenes through, and a shared bandwidth limit for measuring downloads.

    python scripts/serve.py [--port 8765] [--directory dist] [--throttle-mbps 50] [--latency-ms 30]
"""

import argparse
import http.server
import mimetypes
import os
import re
import threading
import time

CHUNK_BYTES = 64 * 1024


class Throttle:
    """Paces all responses together, as a single network link would."""

    def __init__(self, megabits_per_second):
        self.bytes_per_second = megabits_per_second * 1e6 / 8 if megabits_per_second else 0
        self.lock = threading.Lock()
        self.next_free = 0.0

    def wait(self, byte_count):
        if not self.bytes_per_second:
            return
        with self.lock:
            now = time.monotonic()
            start = max(now, self.next_free)
            self.next_free = start + byte_count / self.bytes_per_second
        time.sleep(max(0.0, self.next_free - time.monotonic()))


class Handler(http.server.BaseHTTPRequestHandler):
    # HTTP/1.1 keeps connections open, like a real static host.
    protocol_version = "HTTP/1.1"
    directory = "dist"
    throttle = Throttle(0)
    latency = 0.0

    def log_message(self, format, *args):
        pass

    def resolve(self):
        path = self.path.split("?", 1)[0].split("#", 1)[0]
        parts = [p for p in path.split("/") if p and p != "."]
        # Nothing above the directory: on Windows a backslash or a drive would climb out too.
        if any(p == ".." or "\\" in p or ":" in p for p in parts):
            return None
        full = os.path.join(self.directory, *parts)
        if os.path.isdir(full):
            full = os.path.join(full, "index.html")
        return full if os.path.isfile(full) else None

    def do_HEAD(self):
        self.respond(send_body=False)

    def do_GET(self):
        self.respond(send_body=True)

    def respond_many(self, full, ranges):
        """Serves many parts of a file as one response, one after another. The browser allows six
        connections per host, so asking for pages one at a time caps a local scene at about a
        thousand pages a second, however fast the disk is."""
        size = os.path.getsize(full)
        spans = []
        for part in ranges.split(","):
            first, last = (int(number) for number in part.split("-"))
            if not 0 <= first <= last < size:
                self.send_error(416)
                return
            spans.append((first, last))
        total = sum(last - first + 1 for first, last in spans)
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(total))
        self.send_header("X-Solstice-Ranges", "1")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        with open(full, "rb") as file:
            for first, last in spans:
                file.seek(first)
                # A part may be most of the file: never more than a chunk of it in memory.
                remaining = last - first + 1
                while remaining > 0:
                    chunk = file.read(min(CHUNK_BYTES, remaining))
                    if not chunk:
                        return
                    self.throttle.wait(len(chunk))
                    try:
                        self.wfile.write(chunk)
                    except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                        return
                    remaining -= len(chunk)

    def respond(self, send_body):
        time.sleep(self.latency)
        full = self.resolve()
        if full is None:
            self.send_error(404)
            return
        ranges = self.headers.get("X-Solstice-Ranges")
        if ranges and send_body:
            try:
                self.respond_many(full, ranges)
            except ValueError:
                self.send_error(400)
            return
        stat = os.stat(full)
        size = stat.st_size
        first, last = 0, size - 1
        status = 200
        match = re.fullmatch(r"bytes=(\d*)-(\d*)", self.headers.get("Range", ""))
        if match and (match[1] or match[2]):
            if match[1]:
                first = int(match[1])
                last = min(int(match[2]), size - 1) if match[2] else size - 1
            else:
                first = max(size - int(match[2]), 0)
            if first > last:
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{size}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            status = 206

        self.send_response(status)
        kind = mimetypes.guess_type(full)[0] or "application/octet-stream"
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(last - first + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", f'"{stat.st_mtime_ns:x}-{size:x}"')
        self.send_header("Cache-Control", "no-store")
        if status == 206:
            self.send_header("Content-Range", f"bytes {first}-{last}/{size}")
        self.end_headers()
        if not send_body:
            return

        with open(full, "rb") as file:
            file.seek(first)
            remaining = last - first + 1
            while remaining > 0:
                chunk = file.read(min(CHUNK_BYTES, remaining))
                if not chunk:
                    break
                self.throttle.wait(len(chunk))
                try:
                    self.wfile.write(chunk)
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    return
                remaining -= len(chunk)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--directory", default="dist")
    parser.add_argument("--throttle-mbps", type=float, default=0)
    parser.add_argument("--latency-ms", type=float, default=0)
    args = parser.parse_args()

    Handler.directory = args.directory
    Handler.throttle = Throttle(args.throttle_mbps)
    Handler.latency = args.latency_ms / 1000
    mimetypes.add_type("application/wasm", ".wasm")
    mimetypes.add_type("text/javascript", ".js")

    # On Windows, reusing the address would quietly share a port another server is using.
    http.server.ThreadingHTTPServer.allow_reuse_address = os.name != "nt"
    server = http.server.ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"serving {args.directory} on http://127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
