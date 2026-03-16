#!/usr/bin/env python3
"""
图片识别探测脚本

用法：
    1. 运行此脚本: python3 tools/image_probe.py
    2. 在私聊中给机器人发送一张图片
    3. 脚本自动抓取图片信息并测试各个 API

监听端口: 0.0.0.0:18888
输出目录: /tmp/image_probe/

注意：使用前请先停止 napcat_probe.py（占用同一端口）
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import requests
import base64
import os
from datetime import datetime

OUT = "/tmp/image_probe"
os.makedirs(OUT, exist_ok=True)

# 配置：修改为 NapCat 实际地址
NAPCAT_URL = "http://127.0.0.1:3000"
NAPCAT_TOKEN = None
DEEPSEEK_KEY = None

# 从 runtime.toml 读取配置
try:
    with open("runtime.toml") as f:
        in_napcat = False
        in_llm = False
        for line in f:
            line = line.strip()

            # 检测段
            if line == "[napcat]":
                in_napcat = True
                in_llm = False
                continue
            elif line == "[llm]":
                in_llm = True
                in_napcat = False
                continue
            elif line.startswith("["):
                in_napcat = False
                in_llm = False
                continue

            # 解析字段
            if "=" in line and not line.startswith("#"):
                key, value = line.split("=", 1)
                key = key.strip()
                value = value.strip().strip('"').strip("'")

                if in_napcat:
                    if key == "url":
                        NAPCAT_URL = value
                    elif key == "token":
                        NAPCAT_TOKEN = value if value else None
                elif in_llm:
                    if key == "api_key":
                        DEEPSEEK_KEY = value
except:
    pass

def napcat_api(endpoint, payload):
    """调用 NapCat API"""
    url = f"{NAPCAT_URL}/{endpoint.lstrip('/')}"
    headers = {}
    if NAPCAT_TOKEN:
        headers["Authorization"] = f"Bearer {NAPCAT_TOKEN}"
    try:
        resp = requests.post(url, json=payload, headers=headers, timeout=30)
        return resp.json()
    except Exception as e:
        print(f"[!] NapCat API 失败: {e}")
        return None

def test_get_image(file_id):
    """测试 get_image"""
    print(f"\n[1] 测试 get_image API")
    print(f"    file_id: {file_id}")
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
        print(f"\n[DEBUG] 收到 POST 请求: {self.path}")

        cl = self.headers.get('Content-Length')
        te = (self.headers.get('Transfer-Encoding') or '').lower()

        print(f"[DEBUG] Content-Length: {cl}, Transfer-Encoding: {te}")

        # 处理不同的传输方式
        if cl:
            raw = self.rfile.read(int(cl))
        elif 'chunked' in te:
            print(f"[DEBUG] 使用 chunked 传输")
            chunks = []
            while True:
                size_line = self.rfile.readline().strip()
                if not size_line:
                    break
                size = int(size_line, 16)
                if size == 0:
                    break
                chunks.append(self.rfile.read(size))
                self.rfile.read(2)  # 读取 \r\n
            raw = b''.join(chunks)
        else:
            raw = self.rfile.read(65536)

        print(f"[DEBUG] 读取字节数: {len(raw)}")

        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

        if not raw:
            print("[DEBUG] body 为空，跳过")
            return

        try:
            data = json.loads(raw)
            print(f"[DEBUG] JSON 解析成功")
        except Exception as e:
            print(f"[!] JSON 解析失败: {e}")
            return

        # 保存原始消息用于调试
        ts = datetime.now().strftime('%H%M%S_%f')
        with open(f"{OUT}/{ts}_raw.json", 'w') as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        print(f"[DEBUG] 原始消息已保存: {OUT}/{ts}_raw.json")

        # 根据 OneBot v11 协议解析
        # post_type 可能是 "message" 或 "message_sent"
        post_type = data.get("post_type", "")
        print(f"[DEBUG] post_type: {post_type}")

        if post_type not in ["message", "message_sent"]:
            print(f"[*] 跳过非消息事件: post_type={post_type}")
            return

        # 提取 message 数组（MessageSegment 列表）
        message = data.get("message", [])
        if not isinstance(message, list):
            print(f"[!] message 字段不是数组: {type(message)}")
            return

        user_id = data.get("user_id", "?")
        message_type = data.get("message_type", "?")

        print(f"\n[*] 收到消息: {message_type} from {user_id}")
        print(f"[DEBUG] message 数组长度: {len(message)}")

        # 遍历消息段，查找图片
        found_image = False
        for idx, seg in enumerate(message):
            print(f"[DEBUG] 消息段 {idx}: type={seg.get('type', '?')}")

            if not isinstance(seg, dict):
                continue

            seg_type = seg.get("type", "")
            seg_data = seg.get("data", {})

            if seg_type == "image":
                found_image = True

                # 提取 file 和 url 字段（对应 MessageSegment::image_file 和 image_url）
                file_id = seg_data.get("file", "")
                url = seg_data.get("url", "")

                print("\n" + "="*60)
                print(f"[图片段] type=image")
                print(f"  data.file: {file_id}")
                print(f"  data.url: {url}")
                print("="*60)

                # 测试各个 API
                if file_id:
                    img_info = test_get_image(file_id)
                    image_b64 = test_download_stream(file_id)

                    if url:
                        test_ocr(url)

                    if image_b64:
                        test_deepseek_vision(image_b64)

                    print("\n" + "="*60)
                    print("测试完成")
                    print("="*60)
                else:
                    print("[!] file 字段为空，无法测试")

        if not found_image:
            print("[*] 消息中无图片段")

    def log_message(self, *a):
        pass

print("="*60)
print("图片识别探测脚本")
print("="*60)
print(f"[配置] NapCat URL: {NAPCAT_URL}")
print(f"[配置] NapCat Token: {'已设置' if NAPCAT_TOKEN else '未设置'}")

if DEEPSEEK_KEY:
    print(f"[配置] DeepSeek API Key: {DEEPSEEK_KEY[:10]}...")
else:
    print(f"[配置] DeepSeek API Key: 未找到，将跳过 Vision 测试")

print(f"[配置] 监听端口: 0.0.0.0:18888")
print(f"[配置] 输出目录: {OUT}/")
print("="*60)
print("[提示] 请先停止 napcat_probe.py，然后发送图片...")
print()

HTTPServer(('0.0.0.0', 18888), Handler).serve_forever()
