#!/usr/bin/env python3
"""
Gemini Vision API 测试脚本

用法：
    python3 tools/test_gemini.py <图片路径>

示例：
    python3 tools/test_gemini.py /tmp/image_probe/013535.jpg
"""

import sys
import base64
import requests
import json

# Gemini API 配置
# 获取免费 API Key: https://makersuite.google.com/app/apikey
GEMINI_API_KEY = ""  # 需要替换
GEMINI_MODEL = "gemini-2.5-flash"  # 最新的多模态模型

def test_gemini_vision(image_path, prompt="请详细描述这张图片的内容"):
    """测试 Gemini Vision API"""

    if GEMINI_API_KEY == "YOUR_API_KEY_HERE":
        print("[!] 请先设置 GEMINI_API_KEY")
        print("[!] 获取地址: https://makersuite.google.com/app/apikey")
        return None

    # 读取图片并转 base64
    try:
        with open(image_path, 'rb') as f:
            image_data = base64.b64encode(f.read()).decode()
    except Exception as e:
        print(f"[!] 读取图片失败: {e}")
        return None

    # 构造请求
    url = f"https://generativelanguage.googleapis.com/v1beta/models/{GEMINI_MODEL}:generateContent?key={GEMINI_API_KEY}"

    payload = {
        "contents": [{
            "parts": [
                {"text": prompt},
                {
                    "inline_data": {
                        "mime_type": "image/jpeg",
                        "data": image_data
                    }
                }
            ]
        }]
    }

    print(f"[*] 测试 Gemini Vision API")
    print(f"    模型: {GEMINI_MODEL}")
    print(f"    图片: {image_path}")
    print(f"    提示: {prompt}")
    print()

    try:
        resp = requests.post(url, json=payload, timeout=60)

        if resp.status_code != 200:
            print(f"[!] HTTP {resp.status_code}")
            print(f"    {resp.text}")
            return None

        data = resp.json()

        # 提取响应文本
        if "candidates" in data and len(data["candidates"]) > 0:
            candidate = data["candidates"][0]
            if "content" in candidate and "parts" in candidate["content"]:
                text = candidate["content"]["parts"][0].get("text", "")

                print(f"[✓] 识别结果:")
                print(f"{'='*60}")
                print(text)
                print(f"{'='*60}")

                # 显示使用统计
                if "usageMetadata" in data:
                    meta = data["usageMetadata"]
                    print(f"\n[统计]")
                    print(f"    输入 tokens: {meta.get('promptTokenCount', 0)}")
                    print(f"    输出 tokens: {meta.get('candidatesTokenCount', 0)}")
                    print(f"    总计 tokens: {meta.get('totalTokenCount', 0)}")

                return text
            else:
                print(f"[!] 响应格式异常: {data}")
                return None
        else:
            print(f"[!] 无候选结果: {data}")
            return None

    except Exception as e:
        print(f"[!] 请求失败: {e}")
        return None

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("用法: python3 tools/test_gemini.py <图片路径>")
        print("示例: python3 tools/test_gemini.py /tmp/image_probe/013535.jpg")
        sys.exit(1)

    image_path = sys.argv[1]
    test_gemini_vision(image_path)
