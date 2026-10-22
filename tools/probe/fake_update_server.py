"""Fake update-manifest server for the update-check probe.

Answers every GET with a one-field manifest and appends one ASCII log line per request, so the
PowerShell probe can assert that the app really went out and how it identified itself.

Usage: fake_update_server.py <port> <logfile> [version]
"""

import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
LOGFILE = sys.argv[2]
VERSION = sys.argv[3] if len(sys.argv) > 3 else "9.9.9"

MANIFEST = '{{"version": "{v}", "url": "https://example.com/cuter/download", "notes": "probe fixture"}}'


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 - http.server spells it this way
        body = MANIFEST.format(v=VERSION).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
        user_agent = self.headers.get("User-Agent", "")
        with open(LOGFILE, "a", encoding="utf-8") as handle:
            handle.write(f"GET {self.path} UA={user_agent}\n")

    def log_message(self, *args):
        pass  # the access log would land on stdout; the probe reads the file instead


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
