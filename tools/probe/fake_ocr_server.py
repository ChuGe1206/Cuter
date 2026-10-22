"""A fake PaddleOCR `hubserving` endpoint, for the online-OCR end-to-end probe.

Usage: fake_ocr_server.py <port> <request-log>
It answers every POST with a fixed two-block result and appends a few facts about the
request (path, body size, whether the multipart `image` field and a base64 PNG header are
present) to the log, so the probe can prove the app really posted an image.

`POST /ocr?rotate=1` answers with a **rotated** quad as well (a slanted box plus a vertical
one), which is what exercises the quad-aware highlight in the OCR panel. `POST /ocr?vertical=1`
answers with three vertical columns in the wrong order, which is what exercises the
vertical reading-order pass.
"""

import json
import pathlib
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(sys.argv[1])
LOG = pathlib.Path(sys.argv[2])

PAYLOAD = {
    "status": "000",
    "results": [
        [
            ["在线接口识别成功", 0.99, [[10, 10], [220, 10], [220, 40], [10, 40]]],
            ["HTTP OCR online", 0.95, [[10, 60], [200, 60], [200, 90], [10, 90]]],
        ]
    ],
}

# 斜着的一块 + 竖排的一块（竖排框也是旋转四边形，只是转 90°）
ROTATED_PAYLOAD = {
    "status": "000",
    "results": [
        [
            ["在线接口识别成功", 0.99, [[10, 10], [220, 10], [220, 40], [10, 40]]],
            ["斜排文本 rotated text", 0.94, [[120, 150], [400, 96], [416, 176], [136, 230]]],
            ["竖排", 0.92, [[470, 60], [505, 52], [540, 250], [505, 258]]],
        ]
    ],
}

# 三列竖排，引擎顺序故意给成"左→右→中"：重排之后该读成 右列、中列、左列。
VERTICAL_PAYLOAD = {
    "status": "000",
    "results": [
        [
            ["左列", 0.95, [[180, 60], [230, 60], [230, 300], [180, 300]]],
            ["右列", 0.96, [[420, 60], [470, 60], [470, 300], [420, 300]]],
            ["中列", 0.94, [[300, 60], [350, 60], [350, 300], [300, 300]]],
        ]
    ],
}


class Handler(BaseHTTPRequestHandler):
    def _send(self, payload: bytes) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:  # readiness probe
        self._send(json.dumps({"status": "000", "results": []}).encode())

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        rotated = "rotate=1" in self.path
        vertical = "vertical=1" in self.path
        with LOG.open("a", encoding="utf-8") as handle:
            handle.write(f"path={self.path} bytes={len(body)}\n")
            handle.write(f"has_image_field={b'name=\"image\"' in body}\n")
            handle.write(f"has_png_base64={b'iVBOR' in body}\n")
            handle.write(f"rotated={rotated}\n")
            handle.write(f"vertical={vertical}\n")
        if vertical:
            payload = VERTICAL_PAYLOAD
        elif rotated:
            payload = ROTATED_PAYLOAD
        else:
            payload = PAYLOAD
        self._send(json.dumps(payload).encode())

    def log_message(self, *args) -> None:  # keep the console quiet
        pass


HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
