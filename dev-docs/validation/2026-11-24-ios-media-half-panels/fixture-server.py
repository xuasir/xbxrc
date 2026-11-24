from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from pathlib import Path
import time
base = Path(__file__).parent / "fixtures"
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        clip = self.path == "/clip.mp4"
        payload = (base / ("clip.mp4" if clip else "image.png")).read_bytes()
        if not clip: payload += bytes(4 * 1024 * 1024)
        status = 200
        content_range = None
        if clip and self.headers.get("Range"):
            total = len(payload)
            request_range = self.headers["Range"].replace("bytes=", "").split("-", 1)
            start = int(request_range[0] or "0")
            end = min(total - 1, int(request_range[1] or str(total - 1)))
            payload = payload[start:end+1]
            status = 206
            content_range = f"bytes {start}-{end}/{total}"
        self.send_response(status)
        if clip: self.send_header("Accept-Ranges", "bytes")
        if content_range: self.send_header("Content-Range", content_range)
        self.send_header("Content-Type", "video/mp4" if clip else "image/png")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        try:
            for offset in range(0, len(payload), 32768):
                self.wfile.write(payload[offset:offset+32768])
                self.wfile.flush()
                if not clip: time.sleep(0.16)
        except (BrokenPipeError, ConnectionResetError): pass
ThreadingHTTPServer(("127.0.0.1", 18764), Handler).serve_forever()
