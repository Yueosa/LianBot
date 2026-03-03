#!/usr/bin/env python3
"""
NapCat 推送结构探测 (T1)
用法:
    python3 tools/napcat_probe.py

说明:
    启动一个本地 HTTP 监听器（默认 0.0.0.0:18888），接收 NapCat 的 POST 推送。
    建议将 NapCat 反向 HTTP 上报地址指向本机该端口，用于采集真实事件样本。

输出:
    1) 控制台打印：消息序号、昵称（card 优先，否则 nickname）、raw_message 摘要
    2) 样本文件：/tmp/napcat_probe/<timestamp>.json
    3) 异常原文：JSON 解析失败时保存到 /tmp/napcat_probe/<timestamp>_raw.bin
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json, datetime, os

OUT = "/tmp/napcat_probe"
os.makedirs(OUT, exist_ok=True)
counter = 0

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        global counter
        cl = self.headers.get('Content-Length')
        te = (self.headers.get('Transfer-Encoding') or '').lower()

        if cl:
            raw = self.rfile.read(int(cl))
        elif 'chunked' in te:
            chunks = []
            while True:
                size_line = self.rfile.readline().strip()
                size = int(size_line, 16)
                if size == 0:
                    break
                chunks.append(self.rfile.read(size))
                self.rfile.read(2)
            raw = b''.join(chunks)
        else:
            raw = self.rfile.read(65536)

        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

        if not raw:
            print("[!] 空 body，跳过")
            return

        try:
            data = json.loads(raw)
        except Exception as e:
            ts = datetime.datetime.now().strftime('%H%M%S_%f')
            with open(f"{OUT}/{ts}_raw.bin", 'wb') as f:
                f.write(raw)
            print(f"[!] JSON 解析失败: {e}，原始数据已保存")
            return

        counter += 1
        ts = datetime.datetime.now().strftime('%H%M%S_%f')
        path = f"{OUT}/{ts}.json"
        with open(path, 'w') as f:
            json.dump(data, f, ensure_ascii=False, indent=2)

        sender = data.get('sender', {})
        nick = sender.get('card') or sender.get('nickname', '?')
        raw_msg = data.get('raw_message', '')[:60].replace('\n', ' ')
        print(f"[{counter:03d}] nick={nick!r} raw={raw_msg!r}")
        print(f"      -> {path}")

    def log_message(self, *a): pass

print(f"监听 0.0.0.0:18888 → {OUT}/")
HTTPServer(('0.0.0.0', 18888), H).serve_forever()
