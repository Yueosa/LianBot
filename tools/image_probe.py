#!/usr/bin/env python3
"""
图片识别探测脚本

用法：
    1. 运行此脚本: python3 tools/image_probe.py
    2. 在私聊中给机器人发送一张图片
    3. 脚本自动抓取图片信息并测试各个 API

监听端口: 0.0.0.0:18889
输出目录: /tmp/image_probe/
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import requests
import base64
import os
from datetime import datetime

OUT = "/tmp/image_probe"
os.makedirs(OUT, exist_ok=True)

NAPCAT_URL = "http://127.0.0.1:3000"
DEEPSEEK_KEY = None

# 从 runtime.toml 读取 API Key
try:
    with open("runtime.toml") as f:
        for line in f:
            if "api_key" in line and "=" in line:
                DEEPSEEK_KEY = line.split("=", 1)[1].strip().strip('"').strip("'")
                break
except:
    pass

def napcat_api(endpoint, payload):
    """调用 NapCat API"""
    url = f"{NAPCAT_URL}/{endpoint.lstrip('/')}"
    try:
        resp = requests.post(url, json=payload, timeout=30)
        return resp.json()
    except Exception as e:
        print(f"[!] NapCat API 失败: {e}")
        return None

def test_get_image(file_id):
    """测试 get_image"""
    print(f"\n[1] 测试 get_image API")
    result = napcat_api("get_image", {"file": file_id})
    if result and result.get("data"):
        data = result["data"]
        print(f"    ✓ file: {data.get('file', 'N/A')}")
        print(f"    ✓ url: {data.get('url', 'N/A')}")
        print(f"    ✓ size: {data.get('file_size', 'N/A')} bytes")
        return data
    else:
        print(f"    ✗ 失败: {result}")
    return None

def test_download_stream(file_id):
    """测试 download_file_image_stream"""
    print(f"\n[2] 测试 download_file_image_stream API")
    result = napcat_api("download_file_image_stream", {
        "file": file_id,
        "chunk_size": 65536
    })
    if result and result.get("data"):
        b64 = result["data"].get("base64", "")
        print(f"    ✓ base64 长度: {len(b64)}")

        # 保存图片
        ts = datetime.now().strftime('%H%M%S')
        path = f"{OUT}/{ts}.jpg"
        with open(path, 'wb') as f:
            f.write(base64.b64decode(b64))
        print(f"    ✓ 已保存: {path}")
        return b64
    else:
        print(f"    ✗ 失败: {result}")
    return None

def test_ocr(image_url):
    """测试 ocr_image"""
    print(f"\n[3] 测试 ocr_image API")
    result = napcat_api("ocr_image", {"image": image_url})
    if result and result.get("data"):
        texts = result["data"].get("texts", [])
        if texts:
            print(f"    ✓ OCR 识别到 {len(texts)} 段文字:")
            for item in texts[:3]:  # 只显示前3条
                print(f"      - {item.get('text', '')}")
        else:
            print(f"    ✓ 无文字")
        return result["data"]
    else:
        print(f"    ✗ 失败: {result}")
    return None

def test_deepseek_vision(image_b64):
    """测试 DeepSeek Vision"""
    if not DEEPSEEK_KEY:
        print(f"\n[4] DeepSeek Vision 测试跳过（未找到 API Key）")
        return None

    print(f"\n[4] 测试 DeepSeek Vision API")

    payload = {
        "model": "deepseek-chat",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "请用一句话描述这张图片"},
                {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{image_b64}"}}
            ]
        }],
        "temperature": 0.7,
        "max_tokens": 300
    }

    try:
        resp = requests.post(
            "https://api.deepseek.com/v1/chat/completions",
            headers={"Authorization": f"Bearer {DEEPSEEK_KEY}", "Content-Type": "application/json"},
            json=payload,
            timeout=60
        )
        data = resp.json()

        if "choices" in data and len(data["choices"]) > 0:
            content = data["choices"][0]["message"]["content"]
            print(f"    ✓ LLM 识别结果:")
            print(f"      {content}")
            return content
        else:
            print(f"    ✗ 响应异常: {data}")
    except Exception as e:
        print(f"    ✗ 请求失败: {e}")
    return None

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        cl = self.headers.get('Content-Length')
        raw = self.rfile.read(int(cl)) if cl else b''

        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

        if not raw:
            return

        try:
            data = json.loads(raw)
        except:
            return

        # 只处理消息事件
        if data.get("post_type") != "message":
            return

        message = data.get("message", [])

        # 查找图片段
        for seg in message:
            if seg.get("type") == "image":
                seg_data = seg.get("data", {})
                file_id = seg_data.get("file", "")
                url = seg_data.get("url", "")

                print("\n" + "="*60)
                print(f"[图片消息] 来自 {data.get('user_id', '?')}")
                print(f"  file: {file_id}")
                print(f"  url: {url}")
                print("="*60)

                # 测试各个 API
                img_info = test_get_image(file_id)
                image_b64 = test_download_stream(file_id)

                if url:
                    test_ocr(url)

                if image_b64:
                    test_deepseek_vision(image_b64)

                print("\n" + "="*60)
                print("测试完成")
                print("="*60)

    def log_message(self, *a):
        pass

if DEEPSEEK_KEY:
    print(f"[✓] DeepSeek API Key: {DEEPSEEK_KEY[:10]}...")
else:
    print(f"[!] 未找到 DeepSeek API Key，将跳过 Vision 测试")

print(f"[*] 监听 0.0.0.0:18889")
print(f"[*] 输出目录: {OUT}/")
print(f"[*] 请在 NapCat 配置中添加反向 HTTP 上报: http://127.0.0.1:18889")
print(f"[*] 然后在私聊中发送图片...")
print()

HTTPServer(('0.0.0.0', 18889), Handler).serve_forever()
