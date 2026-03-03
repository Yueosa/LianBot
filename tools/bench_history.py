#!/usr/bin/env python3
"""
get_group_msg_history 性能基准测试 (T2)
用法:
  python3 tools/bench_history.py --url http://127.0.0.1:3000 --group <GROUP_ID>
  python3 tools/bench_history.py --url http://127.0.0.1:3000 --group <GROUP_ID> --token <TOKEN>
  python3 tools/bench_history.py --url http://127.0.0.1:3000 --group <GROUP_ID> --reps 5

输出:
  每个 count 档位的：响应时间（min/avg/max）、返回实际条数、payload 大小
"""

import argparse
import json
import sys
import time
import urllib.request
import urllib.error


def call_api(base_url: str, token: str, group_id: int, count: int) -> tuple[float, int, int]:
    """
    调用 /get_group_msg_history，返回 (耗时秒, 实际返回条数, payload字节数)
    """
    payload = json.dumps({
        "group_id": group_id,
        "count": count,
        "reverseOrder": False,
    }).encode("utf-8")

    headers = {
        "Content-Type": "application/json",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"

    url = base_url.rstrip("/") + "/get_group_msg_history"
    req = urllib.request.Request(url, data=payload, headers=headers, method="POST")

    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            body = resp.read()
    except urllib.error.URLError as e:
        print(f"  [!] 请求失败: {e}", file=sys.stderr)
        return -1.0, 0, 0
    elapsed = time.perf_counter() - t0

    body_size = len(body)
    try:
        data = json.loads(body)
        messages = data.get("data", {}).get("messages", [])
        actual_count = len(messages)
    except Exception:
        actual_count = -1

    return elapsed, actual_count, body_size


def fmt_size(n: int) -> str:
    if n < 1024:
        return f"{n} B"
    elif n < 1024 * 1024:
        return f"{n / 1024:.1f} KB"
    else:
        return f"{n / 1024 / 1024:.2f} MB"


def main():
    parser = argparse.ArgumentParser(description="get_group_msg_history 性能基准")
    parser.add_argument("--url", default="http://127.0.0.1:3000", help="NapCat HTTP API 地址")
    parser.add_argument("--token", default="", help="Bearer Token（可选）")
    parser.add_argument("--group", type=int, required=True, help="目标群号")
    parser.add_argument("--counts", default="100,500,2000", help="测试的 count 档位，逗号分隔")
    parser.add_argument("--reps", type=int, default=3, help="每个档位重复次数")
    args = parser.parse_args()

    counts = [int(c.strip()) for c in args.counts.split(",")]

    print(f"NapCat: {args.url}")
    print(f"群号:   {args.group}")
    print(f"档位:   {counts}  重复: {args.reps} 次\n")

    print(f"{'count':>6}  {'实际条数':>8}  {'min(ms)':>9}  {'avg(ms)':>9}  {'max(ms)':>9}  {'响应大小':>10}")
    print("-" * 62)

    for count in counts:
        times = []
        actual = 0
        size = 0
        for rep in range(args.reps):
            elapsed, actual_count, body_size = call_api(args.url, args.token, args.group, count)
            if elapsed < 0:
                break
            times.append(elapsed * 1000)  # 转毫秒
            actual = actual_count
            size = body_size
            # 每次请求间隔 500ms，避免 NapCat 限速
            if rep < args.reps - 1:
                time.sleep(0.5)

        if not times:
            print(f"{count:>6}  {'ERROR':>8}")
            continue

        t_min = min(times)
        t_avg = sum(times) / len(times)
        t_max = max(times)
        print(f"{count:>6}  {actual:>8}  {t_min:>9.1f}  {t_avg:>9.1f}  {t_max:>9.1f}  {fmt_size(size):>10}")

    print()
    print("说明:")
    print("  实际条数 <= count（群消息不足时返回实际有的条数）")
    print("  响应大小 = 最后一次请求的 HTTP body 字节数")
    print("  此数据用于评估 smy/fetcher 内存占用和 fallback 策略")


if __name__ == "__main__":
    main()
