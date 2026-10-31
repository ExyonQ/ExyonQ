#!/usr/bin/env python3
"""Deterministic upstream for the NG-18 gunzip differential corpus.

Serves gzip and plain bodies with stable bytes so NGINX and ExyonQ can be
compared byte-for-byte. Test-only; never ship.
"""
import gzip
import hashlib
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SMALL = (b"ng18-gunzip-body\n" * 4096)  # 64 KiB
LARGE = (b"ng18-large-body-0123456789\n" * 65536)  # ~1.8 MiB
PLAIN = b"ng18-plain-body\n" * 64


def gz(data: bytes) -> bytes:
    return gzip.compress(data, mtime=0)


ROUTES = {
    "/gzip": (gz(SMALL), SMALL, "gzip"),
    "/gzip-large": (gz(LARGE), LARGE, "gzip"),
    "/plain": (PLAIN, PLAIN, None),
    "/invalid": (b"not-a-gzip-stream", b"not-a-gzip-stream", "gzip"),
    "/empty": (gz(b""), b"", "gzip"),
}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        for prefix in ("/off", "/on"):
            if path.startswith(prefix + "/"):
                path = path[len(prefix):]
                break
        if path not in ROUTES:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        wire, _, encoding = ROUTES[path]
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Cache-Control", "public, max-age=60")
        self.send_header("ETag", '"ng18-%s-v1"' % path.strip("/"))
        self.send_header("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT")
        if encoding:
            self.send_header("Content-Encoding", encoding)
        self.send_header("Content-Length", str(len(wire)))
        self.end_headers()
        self.wfile.write(wire)

    def do_HEAD(self):
        self.do_GET()

    def log_message(self, *_args):
        pass


class Server(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 128


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 19090
    server = Server(("0.0.0.0", port), Handler)
    print(f"ng18 upstream on :{port}", flush=True)
    server.serve_forever()
