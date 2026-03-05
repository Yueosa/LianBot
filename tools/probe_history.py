#!/usr/bin/env python3
"""
get_group_msg_history 边界探测 (T3)

目标：
  1. count 上限探测 — 超过 2000 时 NapCat 是截断、报错还是正常返回？
  2. message_seq 分页探测 — 是否可以用最早消息的 seq 向前翻页回溯更深历史？
  3. 时间回溯深度 — 分页到底后，最早能拿到多久之前的消息？
  4. reverse_order 行为确认 — 字段语义验证

用法:
  # 最小调用（仅需群号）
  python3 tools/probe_history.py --url http://127.0.0.1:3000 --group <GROUP_ID>

  # 完整参数
  python3 tools/probe_history.py --url http://127.0.0.1:3000 --group <GROUP_ID> \\
      --token <TOKEN> --max-pages 20 --page-size 2000 --interval 0.8

输出:
  每个探测阶段的结构化结果，最终汇总 API 的实际能力边界
"""

import argparse
import json
import sys
import time
import datetime
import urllib.request
import urllib.error
from typing import Optional


# ── HTTP 辅助 ─────────────────────────────────────────────────────────────────

def post(base_url: str, token: str, endpoint: str, payload: dict, timeout: int = 30):
    """发送一次 POST 请求，返回 (原始body, 耗时秒)，失败返回 (None, -1)"""
    data = json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    url = base_url.rstrip("/") + "/" + endpoint.lstrip("/")
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")

    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read()
        return body, time.perf_counter() - t0
    except urllib.error.URLError as e:
        print(f"  [!] 请求失败: {e}", file=sys.stderr)
        return None, -1.0


def fetch_history(base_url: str, token: str, group_id: int,
                  count: int, message_seq: Optional[int] = None,
                  reverse_order: bool = False) -> tuple[list, float, int]:
    """
    调用 get_group_msg_history，返回 (messages列表, 耗时秒, body字节数)
    """
    payload = {
        "group_id": group_id,
        "count": count,
        "reverse_order": reverse_order,
        "reverseOrder": reverse_order,
        "disable_get_url": True,    # 不需要图片 URL，减小 payload
        "parse_mult_msg": False,    # 不展开合并消息
        "quick_reply": False,
    }
    if message_seq is not None:
        payload["message_seq"] = message_seq

    body, elapsed = post(base_url, token, "get_group_msg_history", payload)
    if body is None:
        return [], elapsed, 0

    body_size = len(body)
    try:
        data = json.loads(body)
        msgs = data.get("data", {}).get("messages", [])
        return msgs, elapsed, body_size
    except Exception as e:
        print(f"  [!] JSON 解析失败: {e}", file=sys.stderr)
        return [], elapsed, body_size


# ── 辅助函数 ──────────────────────────────────────────────────────────────────

def ts_to_str(ts: Optional[int]) -> str:
    if ts is None:
        return "N/A"
    return datetime.datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M:%S")


def age_str(ts: Optional[int]) -> str:
    """返回相对于现在的时间差，如 '3天前'"""
    if ts is None:
        return "N/A"
    delta = int(time.time()) - ts
    if delta < 0:
        return "未来?"
    m, s = divmod(delta, 60)
    h, m = divmod(m, 60)
    d, h = divmod(h, 24)
    if d > 0:
        return f"{d}天{h}小时前"
    elif h > 0:
        return f"{h}小时{m}分前"
    else:
        return f"{m}分{s}秒前"


def get_seq(msg: dict) -> Optional[int]:
    """从消息对象中提取 message_seq"""
    return msg.get("message_seq") or msg.get("seq")


def get_ts(msg: dict) -> Optional[int]:
    return msg.get("time")


def fmt_size(n: int) -> str:
    if n < 1024:
        return f"{n} B"
    elif n < 1024 * 1024:
        return f"{n / 1024:.1f} KB"
    else:
        return f"{n / 1024 / 1024:.2f} MB"


def section(title: str):
    print(f"\n{'─' * 60}")
    print(f"  {title}")
    print(f"{'─' * 60}")


# ── 探测阶段 ──────────────────────────────────────────────────────────────────

def probe_count_ceiling(base_url: str, token: str, group_id: int, interval: float):
    """
    阶段 1：count 上限探测
    测试 count = 100 / 500 / 1000 / 2000 / 3000 / 5000
    观察：API 是截断返回 2000、原样返回、还是报错
    """
    section("阶段 1 — count 上限探测")
    print(f"  {'count':>6}  {'实际返回':>8}  {'耗时(ms)':>10}  {'body大小':>10}  {'最早时间'}")
    print(f"  {'-'*6}  {'-'*8}  {'-'*10}  {'-'*10}  {'-'*22}")

    ceiling_result = {}
    for count in [100, 500, 1000, 2000, 3000, 5000]:
        msgs, elapsed, bsize = fetch_history(base_url, token, group_id, count)
        actual = len(msgs)
        oldest_ts = get_ts(msgs[0]) if msgs else None
        tag = ""
        if actual == 0:
            tag = " [空/报错]"
        elif actual < count:
            tag = f" [服务端条数不足或被截断]"
        elif actual == 2000 and count > 2000:
            tag = " [★ 疑似硬上限=2000]"

        print(f"  {count:>6}  {actual:>8}  {elapsed*1000:>10.1f}  {fmt_size(bsize):>10}  {ts_to_str(oldest_ts)}{tag}")
        ceiling_result[count] = {"actual": actual, "oldest_ts": oldest_ts}
        time.sleep(interval)

    return ceiling_result


def probe_pagination(base_url: str, token: str, group_id: int,
                     page_size: int, max_pages: int, interval: float):
    """
    阶段 2：message_seq 分页回溯深度探测
    首次不传 message_seq，从最新开始；后续取每批最早消息的 seq 继续向前翻
    直到：收到空结果 / 连续返回条数 < page_size（无更多历史）/ 到达 max_pages
    """
    section(f"阶段 2 — message_seq 分页回溯（每页 {page_size} 条，最多 {max_pages} 页）")

    # NapCat 实际返回升序（最旧→最新）：msgs[0]=最旧, msgs[-1]=最新
    # 向前翻页需要取 msgs[0].seq（最旧消息的 seq）作为下一页起点
    print(f"  {'页':>4}  {'返回条数':>8}  {'耗时(ms)':>10}  {'最早时间（本页）':>22}  {'最新时间（本页）':>22}  {'累计回溯'}")
    print(f"  {'-'*4}  {'-'*8}  {'-'*10}  {'-'*22}  {'-'*22}  {'-'*16}")

    pages = []
    current_seq: Optional[int] = None
    global_newest_ts: Optional[int] = None
    total_messages = 0
    stopped_reason = "达到最大页数限制"

    for page_num in range(1, max_pages + 1):
        msgs, elapsed, _ = fetch_history(
            base_url, token, group_id,
            count=page_size,
            message_seq=current_seq,
        )
        actual = len(msgs)
        total_messages += actual

        if actual == 0:
            stopped_reason = f"第 {page_num} 页返回空（无更多历史）"
            print(f"  {page_num:>4}  {'0':>8}  —  [无更多历史，停止]")
            break

        # API 返回升序（time 旧→新）：msgs[0]=最旧, msgs[-1]=最新
        oldest_ts  = get_ts(msgs[0])
        newest_ts  = get_ts(msgs[-1])
        oldest_seq = get_seq(msgs[0])   # 向前翻页用最旧消息的 seq

        if page_num == 1:
            global_newest_ts = newest_ts

        # 累计回溯：从全局最新到本页最旧的跨度
        if global_newest_ts and oldest_ts:
            delta_h = (global_newest_ts - oldest_ts) / 3600
            if delta_h >= 48:
                depth_str = f"{delta_h/24:.1f}天"
            else:
                depth_str = f"{delta_h:.1f}小时"
        else:
            depth_str = "N/A"

        ended = ""
        if actual < page_size:
            ended = " [★ 最后一页，历史已穷尽]"
            stopped_reason = f"第 {page_num} 页实际返回 {actual} < {page_size}，历史已穷尽"

        print(f"  {page_num:>4}  {actual:>8}  {elapsed*1000:>10.1f}  {ts_to_str(oldest_ts):>22}  {ts_to_str(newest_ts):>22}  {depth_str}{ended}")

        pages.append({
            "page": page_num,
            "count": actual,
            "newest_ts": newest_ts,
            "oldest_ts": oldest_ts,
            "oldest_seq": oldest_seq,
        })

        if actual < page_size:
            break

        # 下一页：传最旧消息的 seq，让 NapCat 返回该 seq 之前的消息
        current_seq = oldest_seq
        if current_seq is None:
            stopped_reason = "无法提取 message_seq，停止分页"
            print("  [!] 无法从消息中提取 message_seq，停止")
            break

        time.sleep(interval)

    print(f"\n  停止原因: {stopped_reason}")
    print(f"  总共拉取: {total_messages} 条（不含重复）")
    return pages


def probe_reverse_order(base_url: str, token: str, group_id: int, interval: float):
    """
    阶段 3：验证 reverse_order / reverseOrder 字段行为
    对比 reverse_order=False 和 reverse_order=True 时第一条消息的时间戳
    """
    section("阶段 3 — reverse_order 字段语义验证")

    msgs_f, _, _ = fetch_history(base_url, token, group_id, count=10, reverse_order=False)
    time.sleep(interval)
    msgs_t, _, _ = fetch_history(base_url, token, group_id, count=10, reverse_order=True)

    def first_ts(msgs):
        return get_ts(msgs[0]) if msgs else None
    def last_ts(msgs):
        return get_ts(msgs[-1]) if msgs else None

    ts_f_first = first_ts(msgs_f)
    ts_f_last  = last_ts(msgs_f)
    ts_t_first = first_ts(msgs_t)
    ts_t_last  = last_ts(msgs_t)

    print(f"  reverse_order=false: msgs[0]={ts_to_str(ts_f_first)}  msgs[-1]={ts_to_str(ts_f_last)}")
    print(f"  reverse_order=true:  msgs[0]={ts_to_str(ts_t_first)}  msgs[-1]={ts_to_str(ts_t_last)}")

    if ts_f_first and ts_f_last:
        if ts_f_first < ts_f_last:
            print("  false 排序: 升序（最旧→最新）")
        else:
            print("  false 排序: 降序（最新→最旧）")

    if ts_t_first and ts_t_last:
        if ts_t_first < ts_t_last:
            print("  true  排序: 升序（最旧→最新）")
        else:
            print("  true  排序: 降序（最新→最旧）")

    if ts_f_first and ts_t_first:
        if ts_f_first == ts_t_first and ts_f_last == ts_t_last:
            print("  结论: reverse_order 字段无效（两种设置结果完全相同）")
        elif ts_f_first > ts_t_first:
            print("  结论: false=降序（最新→旧）, true=升序（最旧→新）")
        else:
            print("  结论: false=升序（最旧→新）, true=降序（最新→旧）")
    else:
        print("  结论: 数据不足，无法判断")


def probe_seq_field(base_url: str, token: str, group_id: int):
    """
    阶段 4：检查消息对象中 seq 相关字段实际存在哪些
    """
    section("阶段 4 — 消息对象字段探查（seq / message_seq / message_id）")

    msgs, _, _ = fetch_history(base_url, token, group_id, count=3)
    if not msgs:
        print("  [!] 无消息返回")
        return

    for i, msg in enumerate(msgs[:3]):
        seq        = msg.get("message_seq")
        msg_id     = msg.get("message_id")
        ts         = msg.get("time")
        all_keys   = list(msg.keys())
        print(f"\n  消息 {i+1}:")
        print(f"    message_seq = {seq}")
        print(f"    message_id  = {msg_id}")
        print(f"    time        = {ts}  ({ts_to_str(ts)})")
        print(f"    所有字段    = {all_keys}")


# ── 汇总 ─────────────────────────────────────────────────────────────────────

def summarize(ceiling: dict, pages: list):
    section("汇总 — API 能力边界")

    # count 上限
    actual_at_2000 = ceiling.get(2000, {}).get("actual", 0)
    actual_at_3000 = ceiling.get(3000, {}).get("actual", 0)
    actual_at_5000 = ceiling.get(5000, {}).get("actual", 0)
    # 判断依据：若请求 5000 返回明显多于 2000，说明无 2000 硬上限
    if actual_at_5000 > actual_at_2000 * 1.2:
        print(f"  count 无 2000 硬上限（2000→{actual_at_2000} 条，5000→{actual_at_5000} 条）")
        print(f"  建议: TIME_MODE_COUNT 可提升到 3000~5000")
    elif actual_at_3000 <= 2000 and actual_at_3000 > 0:
        print(f"  count 硬上限: 约 2000 条（请求 3000 实际返回 {actual_at_3000}）")
    else:
        print(f"  count 上限: 数据不足（3000→{actual_at_3000}，5000→{actual_at_5000}，需人工研判）")

    # 分页深度
    if pages:
        total_pages  = len(pages)
        total_msgs   = sum(p["count"] for p in pages)
        newest_ts    = pages[0].get("newest_ts")   # 第1页（最新批次）的最新消息
        oldest_ts    = pages[-1].get("oldest_ts")  # 最后一页的最旧消息
        if newest_ts and oldest_ts:
            depth_secs = newest_ts - oldest_ts
            depth_days = depth_secs / 86400
            print(f"  分页回溯: {total_pages} 页 × ~{pages[0]['count']} 条 = 共 {total_msgs} 条")
            print(f"  时间跨度: {depth_days:.1f} 天  ({ts_to_str(oldest_ts)} → {ts_to_str(newest_ts)})")
            print(f"  最早消息: {ts_to_str(oldest_ts)}  ({age_str(oldest_ts)})")
        else:
            print(f"  分页回溯: {total_pages} 页，时间戳提取失败")
    else:
        print("  分页回溯: 无数据")

    print()
    print("  ─── 对 pool 设计的建议 ──────────────────────────────────────────")
    print("  1. count 单次上限 ≈ 2000 → fetcher TIME_MODE_COUNT 无需超过 2000")
    print("  2. 若分页可用且回溯 ≥ 7 天 → 可考虑初始化时多页 seed 以支持 7d smy")
    print("  3. 若分页可用且回溯 < 7 天 → 7d smy 能力受限于服务端保留策略，无解")
    print("  4. SQLite 的价值：仅在「本次运行内」作内存卸压；跨重启读取不可信")


# ── 入口 ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="get_group_msg_history 边界探测 (T3)",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--url",       default="http://127.0.0.1:3000", help="NapCat HTTP API 地址")
    parser.add_argument("--token",     default="",   help="Bearer Token（可选）")
    parser.add_argument("--group",     type=int, required=True, help="目标群号")
    parser.add_argument("--page-size", type=int, default=2000,  help="分页探测每页条数（默认 2000）")
    parser.add_argument("--max-pages", type=int, default=15,    help="最多翻页数（默认 15）")
    parser.add_argument("--interval",  type=float, default=0.8, help="请求间隔秒数（默认 0.8）")
    parser.add_argument("--skip-ceiling", action="store_true",  help="跳过 count 上限探测（已知时用）")
    parser.add_argument("--skip-pagination", action="store_true", help="跳过分页探测")
    args = parser.parse_args()

    print(f"T3 探测开始")
    print(f"  NapCat: {args.url}")
    print(f"  群号:   {args.group}")
    print(f"  时间:   {datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")

    ceiling = {}
    pages = []

    if not args.skip_ceiling:
        ceiling = probe_count_ceiling(args.url, args.token, args.group, args.interval)

    if not args.skip_pagination:
        pages = probe_pagination(
            args.url, args.token, args.group,
            page_size=args.page_size,
            max_pages=args.max_pages,
            interval=args.interval,
        )

    probe_reverse_order(args.url, args.token, args.group, args.interval)
    probe_seq_field(args.url, args.token, args.group)
    summarize(ceiling, pages)


if __name__ == "__main__":
    main()
