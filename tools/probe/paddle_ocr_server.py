"""真 PaddleOCR 本地服务，用来验证 Cuter 的在线 OCR 通路。

用法（venv 里必须有 paddlepaddle 2.x + paddleocr 2.x）：

    C:\\Users\\ChuGe\\paddle-ocr-venv\\Scripts\\python.exe tools\\probe\\paddle_ocr_server.py --port 8900 --log target\\tmp\\paddle-requests.log

然后让 Cuter 连它（`[ocr] engine = "http"`、`http_endpoint = "http://127.0.0.1:8900/ocr"`）。

协议按 PaddleOCR 老版 hubserving 的形状：请求是 multipart/form-data，字段值是一段
base64 的 PNG；响应是

    {"status": "000", "results": [["文字", 0.98, [[x,y],[x,y],[x,y],[x,y]]], ...]}

这正是 crates/cuter-core/src/ocr/http_ocr.rs 的 parse_paddle_response 认的形状。
为什么不直接用 paddleocr 自带的 hubserving：3.x 已经不再附带它，而 2.9.1 的
`ocr.ocr()` 返回值又正好是这套结构的内层，所以自己开一个 HTTP 服务最省事。

PaddleOCR 3.x 在 Windows CPU 上跑不动（oneDNN + PIR 的
`ConvertPirAttribute2RuntimeAttribute not support ...`），所以这里固定按 2.x 的 API 写。

`--log` 每处理一张图追加一行 JSON（`path`/`bytes`/`blocks`/`texts`），探针据此断言
"请求真的到了、而且认出了字"，不依赖截图。
"""

import argparse
import base64
import json
import os
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOCK = threading.Lock()
OCR = None
LOG_PATH = None


def load_engine(lang: str):
    """懒加载：模型第一次调用才下载/加载。"""
    global OCR
    if OCR is None:
        from paddleocr import PaddleOCR

        OCR = PaddleOCR(use_angle_cls=True, lang=lang, show_log=False)
    return OCR


def field_value(body: bytes):
    """从 multipart body 里取第一个字段的值（不认字段名，客户端只有一个字段）。"""
    start = body.find(b"\r\n\r\n")
    if start < 0:
        return None
    start += 4
    end = body.find(b"\r\n--", start)
    if end < 0:
        return None
    return body[start:end]


def to_hubserving(lines):
    """`[[[box, (text, score)], ...]]` → `[text, score, [[x, y] ...]]` 的扁平列表。"""
    blocks = []
    for page in lines or []:
        for item in page or []:
            if not item or len(item) < 2:
                continue
            box, rec = item[0], item[1]
            text = str(rec[0])
            score = float(rec[1])
            points = [[int(round(x)), int(round(y))] for x, y in box]
            blocks.append([text, score, points])
    return blocks


def write_log(entry: dict):
    if not LOG_PATH:
        return
    try:
        with open(LOG_PATH, "a", encoding="utf-8") as out:
            out.write(json.dumps(entry, ensure_ascii=False) + "\n")
    except OSError as err:
        print(f"[ocr] cannot write log: {err}", flush=True)


class Handler(BaseHTTPRequestHandler):
    server_version = "CuterPaddleOCR/1.0"

    def log_message(self, fmt, *args):  # 默认日志太吵
        pass

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status": "000", "results": []}')

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        print(f"[ocr] POST {self.path} content-length={length}", flush=True)
        body = self.rfile.read(length)
        raw = field_value(body)
        if raw is None:
            self.reply(400, {"status": "101", "message": "no multipart field"})
            return
        try:
            data = base64.b64decode(raw, validate=False)
        except Exception as err:  # noqa: BLE001
            self.reply(400, {"status": "102", "message": f"bad base64: {err}"})
            return
        handle, path = tempfile.mkstemp(suffix=".png")
        os.close(handle)
        try:
            with open(path, "wb") as out:
                out.write(data)
            started = time.monotonic()
            with LOCK:
                lines = load_engine(self.server.lang).ocr(path, cls=True)
            blocks = to_hubserving(lines)
            elapsed = int((time.monotonic() - started) * 1000)
        except Exception as err:  # noqa: BLE001
            write_log({"path": self.path, "bytes": len(data), "error": f"{type(err).__name__}: {err}"})
            self.reply(500, {"status": "199", "message": f"{type(err).__name__}: {err}"})
            return
        finally:
            try:
                os.unlink(path)
            except OSError:
                pass
        print(f"[ocr] {len(data)} bytes -> {len(blocks)} blocks in {elapsed} ms", flush=True)
        write_log(
            {
                "path": self.path,
                "bytes": len(data),
                "blocks": len(blocks),
                "elapsed_ms": elapsed,
                "texts": [block[0] for block in blocks],
            }
        )
        # hubserving 的形状是 results[0] 里再放一组块（一层嵌套），不是直接把块摊在 results 里。
        self.reply(200, {"status": "000", "results": [blocks]})

    def reply(self, code: int, payload: dict):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    global LOG_PATH
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8900)
    parser.add_argument("--lang", default="ch")
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--log", default="")
    args = parser.parse_args()
    if args.log:
        LOG_PATH = os.path.abspath(args.log)
        parent = os.path.dirname(LOG_PATH)
        if parent:
            os.makedirs(parent, exist_ok=True)
        open(LOG_PATH, "w", encoding="utf-8").close()
    server = ThreadingHTTPServer((args.bind, args.port), Handler)
    server.lang = args.lang
    print(f"paddle ocr server on http://{args.bind}:{args.port}/ocr (lang={args.lang})", flush=True)
    load_engine(args.lang)  # 先加载，避免第一次请求卡住探针
    print("model ready", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
