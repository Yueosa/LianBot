// smy-bench — LianBot SMY 全管线端到端基准测试工具
//
// 完整模拟真实 SMY 管线的 8 个阶段，逐阶段计时：
//   P1: NapCat 分页拉历史         P2: JSON → ChatMessage 解析
//   P3: 统计分析 (Statistics)     P4: format_for_llm 文本化
//   P5: 3× 并行 LLM              P6: LLM JSON 响应清洗
//   P7: HTML 渲染 (≈1000行模板)  P8: Chrome 无头截图
//
// 用法：
//   cargo run -p smy-bench                       # 读取 runtime.toml + logic.toml
//   cargo run -p smy-bench -- --group 123456789  # 指定群号
//   cargo run -p smy-bench -- --llm-only         # 跳过 NapCat，使用内置示例
//   cargo run -p smy-bench -- --rounds 3         # LLM 重复 3 轮
//   cargo run -p smy-bench -- --model deepseek-reasoner
//   cargo run -p smy-bench -- --no-screenshot    # 跳过截图（无 Chrome 环境）

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use reqwest::Client;
use serde::Deserialize;

// ── 配置 ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct RuntimeToml {
    #[serde(default)]
    napcat: NapcatCfg,
    #[serde(default)]
    bot: BotCfg,
}

#[derive(Debug, Deserialize, Default)]
struct NapcatCfg {
    #[serde(default = "default_napcat_url")]
    url: String,
    #[serde(default)]
    token: Option<String>,
}
fn default_napcat_url() -> String { "http://127.0.0.1:3000".into() }

#[derive(Debug, Deserialize, Default)]
struct BotCfg {
    #[serde(default)]
    initial_groups: Vec<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct LogicToml {
    #[serde(default)]
    smy: SmyCfg,
}

#[derive(Debug, Deserialize, Default)]
struct SmyCfg {
    #[serde(default = "default_screenshot_width")]
    screenshot_width: u32,
    #[serde(default)]
    llm: Option<LlmCfg>,
}
fn default_screenshot_width() -> u32 { 1200 }

#[derive(Debug, Deserialize, Clone)]
struct LlmCfg {
    api_url: String,
    api_key: String,
    #[serde(default = "default_model")]
    model: String,
}
fn default_model() -> String { "deepseek-chat".into() }

// ── ChatMessage（与 LianBot 同构） ───────────────────────────────────────────

#[derive(Debug, Clone)]
struct ChatMessage {
    user_id: i64,
    nickname: String,
    time: i64,
    text: String,
    emoji_count: u32,
    image_count: u32,
    reply_to: Option<i64>,
    at_targets: Vec<i64>,
    face_ids: Vec<String>,
}

// ── Statistics（与 LianBot 同构） ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Statistics {
    message_count: usize,
    participant_count: usize,
    total_characters: usize,
    emoji_count: u32,
    image_count: usize,
    most_active_hour: String,
    hourly_distribution: [u32; 24],
    top_speakers: Vec<(i64, String, usize)>,
    reply_count: usize,
    at_count: usize,
    top_faces: Vec<(String, usize)>,
}

fn analyze(messages: &[ChatMessage]) -> Statistics {
    let mut participants: HashSet<i64> = HashSet::new();
    let mut total_chars: usize = 0;
    let mut emoji_count: u32 = 0;
    let mut image_count: usize = 0;
    let mut reply_count: usize = 0;
    let mut at_count: usize = 0;
    let mut hourly: [u32; 24] = [0; 24];
    let mut speaker_count: HashMap<i64, (String, usize)> = HashMap::new();
    let mut face_freq: HashMap<String, usize> = HashMap::new();

    for msg in messages {
        participants.insert(msg.user_id);
        total_chars += msg.text.chars().count();
        emoji_count += msg.emoji_count;
        image_count += msg.image_count as usize;
        if msg.reply_to.is_some() { reply_count += 1; }
        at_count += msg.at_targets.len();

        for fid in &msg.face_ids {
            *face_freq.entry(fid.clone()).or_insert(0) += 1;
        }

        let dt = chrono::DateTime::from_timestamp(msg.time, 0).unwrap_or_default();
        let hour = dt.format("%H").to_string().parse::<usize>().unwrap_or(0);
        hourly[hour] += 1;

        speaker_count.entry(msg.user_id)
            .and_modify(|(_, c)| *c += 1)
            .or_insert_with(|| (msg.nickname.clone(), 1));
    }

    let peak = hourly.iter().enumerate().max_by_key(|&(_, c)| c).map(|(h, _)| h).unwrap_or(0);
    let most_active_hour = format!("{:02}:00 - {:02}:00", peak, (peak + 1) % 24);

    let mut top_speakers: Vec<(i64, String, usize)> = speaker_count
        .into_iter().map(|(uid, (name, cnt))| (uid, name, cnt)).collect();
    top_speakers.sort_by(|a, b| b.2.cmp(&a.2));
    top_speakers.truncate(10);

    let mut top_faces: Vec<(String, usize)> = face_freq.into_iter().collect();
    top_faces.sort_by(|a, b| b.1.cmp(&a.1));
    top_faces.truncate(3);

    Statistics {
        message_count: messages.len(),
        participant_count: participants.len(),
        total_characters: total_chars,
        emoji_count, image_count, most_active_hour, hourly_distribution: hourly,
        top_speakers, reply_count, at_count, top_faces,
    }
}

// ── LLM 结果（与 LianBot 同构） ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
struct LlmResult {
    #[serde(default)]
    topics: Vec<Topic>,
    #[serde(default)]
    user_titles: Vec<UserTitle>,
    #[serde(default)]
    golden_quotes: Vec<Quote>,
    #[serde(default)]
    relationships: Vec<Relationship>,
}

#[derive(Debug, Clone, Deserialize)]
struct Topic {
    #[serde(default)]
    topic: String,
    #[serde(default)]
    contributors: Vec<String>,
    #[serde(default)]
    detail: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UserTitle {
    #[serde(default)]
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    mbti: String,
    #[serde(default)]
    habit: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Quote {
    #[serde(default)]
    content: String,
    #[serde(default)]
    sender: String,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Relationship {
    #[serde(default, rename = "type")]
    rel_type: String,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    label: String,
    #[serde(default)]
    vibe: String,
    #[serde(default)]
    evidence: Vec<String>,
}

// ── CLI 参数 ─────────────────────────────────────────────────────────────────

struct Args {
    group_id: Option<i64>,
    llm_only: bool,
    no_screenshot: bool,
    rounds: usize,
    model_override: Option<String>,
    days: u32,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut a = Args {
        group_id: None, llm_only: false, no_screenshot: false,
        rounds: 1, model_override: None, days: 7,
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--group" | "-g"    => { i += 1; a.group_id = args.get(i).and_then(|s| s.parse().ok()); }
            "--llm-only"        => { a.llm_only = true; }
            "--no-screenshot"   => { a.no_screenshot = true; }
            "--rounds" | "-r"   => { i += 1; a.rounds = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1); }
            "--model" | "-m"    => { i += 1; a.model_override = args.get(i).cloned(); }
            "--days" | "-d"     => { i += 1; a.days = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(7); }
            "--help" | "-h"     => {
                eprintln!("用法: smy-bench [选项]");
                eprintln!("  -g, --group <ID>    指定群号（默认取 initial_groups 第一个）");
                eprintln!("  -d, --days <N>      获取最近 N 天消息（默认 7）");
                eprintln!("  -r, --rounds <N>    LLM 测试轮数（默认 1）");
                eprintln!("  -m, --model <NAME>  覆盖模型名（如 deepseek-reasoner）");
                eprintln!("  --llm-only          跳过 NapCat，使用内置示例文本测 LLM");
                eprintln!("  --no-screenshot     跳过截图阶段（无 Chrome 环境）");
                std::process::exit(0);
            }
            _ => { eprintln!("未知参数: {}", args[i]); }
        }
        i += 1;
    }
    a
}

// ── 工具函数 ──────────────────────────────────────────────────────────────────

fn find_project_root() -> PathBuf {
    let mut dir = std::env::current_dir().expect("无法获取当前目录");
    loop {
        if dir.join("Cargo.toml").exists() && dir.join("src").exists() {
            return dir;
        }
        if !dir.pop() { break; }
    }
    std::env::current_dir().unwrap()
}

fn load_toml<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("⚠ 解析 {} 失败: {e}", path.display());
            T::default()
        }),
        Err(_) => {
            eprintln!("⚠ 未找到 {}", path.display());
            T::default()
        }
    }
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 { format!("{bytes} B") }
    else if bytes < 1024 * 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) }
    else { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) }
}

fn bar(label: &str, ms: u64, max_ms: u64) {
    let width = 40;
    let filled = if max_ms > 0 { ((ms as f64 / max_ms as f64) * width as f64) as usize } else { 0 };
    let bar_str: String = "█".repeat(filled) + &"░".repeat(width - filled);
    println!("  {label:<14} {bar_str} {ms:>7}ms");
}

fn rss_mb() -> f64 {
    // Linux: 读取 /proc/self/statm 的第二字段 (resident pages)
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages as f64 * 4096.0 / (1024.0 * 1024.0))
        .unwrap_or(0.0)
}

// ── P1: NapCat 历史消息获取 ──────────────────────────────────────────────────

struct HistoryResult {
    messages: Vec<serde_json::Value>,
    total_fetch_time: Duration,
    pages: usize,
    raw_bytes: usize,
}

async fn fetch_history(
    client: &Client,
    napcat: &NapcatCfg,
    group_id: i64,
    cutoff_ts: i64,
) -> Result<HistoryResult, String> {
    let url = format!("{}/get_group_msg_history", napcat.url.trim_end_matches('/'));
    let mut all_messages = Vec::new();
    let mut message_seq: Option<i64> = None;
    let mut pages = 0;
    let mut raw_bytes = 0;
    let start = Instant::now();

    loop {
        pages += 1;
        if pages > 50 { break; }

        let body = serde_json::json!({
            "group_id": group_id,
            "count": 5000,
            "message_seq": message_seq,
            "reverseOrder": false,
        });

        let mut req = client.post(&url).json(&body);
        if let Some(ref tok) = napcat.token {
            if !tok.is_empty() {
                req = req.header("Authorization", format!("Bearer {tok}"));
            }
        }

        let page_start = Instant::now();
        let resp = req.send().await.map_err(|e| format!("NapCat 请求失败: {e}"))?;
        let resp_bytes = resp.bytes().await.map_err(|e| format!("读响应失败: {e}"))?;
        let page_ms = page_start.elapsed().as_millis();
        raw_bytes += resp_bytes.len();

        let parsed: serde_json::Value = serde_json::from_slice(&resp_bytes)
            .map_err(|e| format!("JSON 解析失败: {e}"))?;

        let msgs = parsed["data"]["messages"].as_array().cloned().unwrap_or_default();
        let count = msgs.len();
        println!("    第 {pages:>2} 页: {count:>5} 条  {page_ms:>5}ms  {}", human_size(resp_bytes.len()));

        if count == 0 { break; }

        let earliest_seq = msgs.first().and_then(|m| m["message_seq"].as_i64());
        let earliest_time = msgs.first().and_then(|m| m["time"].as_i64()).unwrap_or(i64::MAX);
        all_messages.extend(msgs);

        if earliest_time <= cutoff_ts { break; }
        if count < 5000 { break; }
        message_seq = earliest_seq;
    }

    Ok(HistoryResult {
        messages: all_messages,
        total_fetch_time: start.elapsed(),
        pages,
        raw_bytes,
    })
}

// ── P2: JSON → ChatMessage 解析（与 LianBot parse_raw_messages 一致）────────

fn parse_raw_messages(raw: &[serde_json::Value], cutoff: Option<i64>) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(raw.len());
    for msg in raw {
        if msg.get("post_type").and_then(|v| v.as_str()) == Some("message_sent") { continue; }

        let time = msg.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
        if let Some(cut) = cutoff {
            if time < cut { continue; }
        }

        let sender = msg.get("sender").cloned().unwrap_or(serde_json::Value::Null);
        let card = sender.get("card").and_then(|v| v.as_str()).unwrap_or("");
        let nickname = sender.get("nickname").and_then(|v| v.as_str()).unwrap_or("未知");
        let display_name = if card.is_empty() { nickname.to_string() } else { card.to_string() };
        let user_id = msg.get("user_id").and_then(|v| v.as_i64()).unwrap_or(0);

        let segments = msg.get("message").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        // 逐 segment 提取字段（与真实 extract_chat_fields 一致）
        let mut text = String::new();
        let mut emoji_count: u32 = 0;
        let mut image_count: u32 = 0;
        let mut reply_to: Option<i64> = None;
        let mut at_targets: Vec<i64> = Vec::new();
        let mut face_ids: Vec<String> = Vec::new();

        for seg in &segments {
            let seg_type = seg.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let data = seg.get("data");
            match seg_type {
                "text" => {
                    if let Some(t) = data.and_then(|d| d.get("text")).and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                }
                "face" => {
                    emoji_count += 1;
                    if let Some(id) = data.and_then(|d| d.get("id")).and_then(|v| v.as_str()) {
                        face_ids.push(id.to_string());
                    } else if let Some(id) = data.and_then(|d| d.get("id")).and_then(|v| v.as_u64()) {
                        face_ids.push(id.to_string());
                    }
                }
                "image" => { image_count += 1; }
                "reply" => {
                    if reply_to.is_none() {
                        reply_to = data.and_then(|d| d.get("id"))
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse().ok())
                            .or_else(|| data.and_then(|d| d.get("id")).and_then(|v| v.as_i64()));
                    }
                }
                "at" => {
                    if let Some(qq) = data.and_then(|d| d.get("qq"))
                        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                    {
                        at_targets.push(qq);
                    }
                }
                _ => {}
            }
        }

        let text = text.trim().to_string();
        if text.is_empty() && image_count == 0 { continue; }

        messages.push(ChatMessage {
            user_id, nickname: display_name, time, text,
            emoji_count, image_count, reply_to, at_targets, face_ids,
        });
    }
    messages
}

// ── P4: format_for_llm（与 LianBot 一致）────────────────────────────────────

fn format_for_llm(messages: &[ChatMessage]) -> String {
    let mut lines = Vec::with_capacity(messages.len());
    for msg in messages {
        if msg.text.is_empty() { continue; }
        let dt = chrono::DateTime::from_timestamp(msg.time, 0).unwrap_or_default();
        let hm = dt.format("%H:%M");
        lines.push(format!("[{hm}] {}: {}", msg.nickname, msg.text));
    }
    lines.join("\n")
}

// ── P5: LLM 调用 ────────────────────────────────────────────────────────────

struct LlmBenchResult {
    name: String,
    connect_ms: u64,
    total_ms: u64,
    req_size: usize,
    resp_size: usize,
    content: String,
    success: bool,
    error: Option<String>,
}

async fn bench_llm_call(
    client: &Client,
    config: &LlmCfg,
    prompt_name: &str,
    prompt: &str,
) -> LlmBenchResult {
    let url = format!("{}/chat/completions", config.api_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": config.model,
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0.7,
        "max_tokens": 4096,
    });
    let body_str = serde_json::to_string(&body).unwrap();
    let req_size = body_str.len();
    let total_start = Instant::now();

    let resp = client.post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .body(body_str)
        .send()
        .await;

    let connect_ms = total_start.elapsed().as_millis() as u64;

    match resp {
        Err(e) => LlmBenchResult {
            name: prompt_name.into(), connect_ms, total_ms: connect_ms,
            req_size, resp_size: 0, content: String::new(),
            success: false, error: Some(format!("连接失败: {e}")),
        },
        Ok(resp) => {
            let status = resp.status();
            let resp_bytes = resp.bytes().await.unwrap_or_default();
            let total_ms = total_start.elapsed().as_millis() as u64;
            let resp_size = resp_bytes.len();

            if !status.is_success() {
                return LlmBenchResult {
                    name: prompt_name.into(), connect_ms, total_ms,
                    req_size, resp_size, content: String::new(),
                    success: false, error: Some(format!("HTTP {status}")),
                };
            }

            let parsed: serde_json::Value = serde_json::from_slice(&resp_bytes).unwrap_or_default();
            let content = parsed["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
            let usage = &parsed["usage"];
            let pt = usage["prompt_tokens"].as_u64().unwrap_or(0);
            let ct = usage["completion_tokens"].as_u64().unwrap_or(0);

            println!("    {prompt_name:<14}  {total_ms:>6}ms  连接={connect_ms}ms  处理={}ms",
                total_ms.saturating_sub(connect_ms));
            println!("      req={} resp={} content={}chars tokens=p{pt}+c{ct}={}",
                human_size(req_size), human_size(resp_size), content.len(), pt + ct);

            LlmBenchResult {
                name: prompt_name.into(), connect_ms, total_ms,
                req_size, resp_size, content, success: true, error: None,
            }
        }
    }
}

// ── P6: LLM JSON 清洗（与 LianBot 一致）────────────────────────────────────

fn clean_llm_json(raw: &str) -> String {
    let s = raw.trim();
    // 去掉 markdown 代码块
    let s = if s.starts_with("```") {
        let inner = s.trim_start_matches("```json").trim_start_matches("```");
        inner.trim_end_matches("```").trim()
    } else { s };
    // 去 BOM + 控制字符
    s.replace('\u{feff}', "")
     .chars()
     .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
     .collect()
}

fn parse_llm_topics(content: &str) -> Vec<Topic> {
    let cleaned = clean_llm_json(content);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

fn parse_llm_titles(content: &str) -> (Vec<UserTitle>, Vec<Quote>) {
    let cleaned = clean_llm_json(content);
    #[derive(Deserialize, Default)]
    struct TitleResp {
        #[serde(default)]
        user_titles: Vec<UserTitle>,
        #[serde(default)]
        golden_quotes: Vec<Quote>,
    }
    let r: TitleResp = serde_json::from_str(&cleaned).unwrap_or_default();
    (r.user_titles, r.golden_quotes)
}

fn parse_llm_relationships(content: &str) -> Vec<Relationship> {
    let cleaned = clean_llm_json(content);
    serde_json::from_str(&cleaned).unwrap_or_default()
}

// ── P7: HTML 渲染（复刻真实 ~1000 行模板，同等复杂度）──────────────────────

fn render_html(
    stats: &Statistics,
    llm: &LlmResult,
    group_name: &str,
    messages: &[ChatMessage],
) -> String {
    let mut name_to_uid: HashMap<String, i64> = HashMap::new();
    for msg in messages {
        name_to_uid.entry(msg.nickname.clone()).or_insert(msg.user_id);
    }

    let now = chrono::Utc::now();
    let date_str = now.format("%Y年%m月%d日").to_string();
    let datetime_str = now.format("%Y-%m-%d %H:%M").to_string();

    let mut html = String::with_capacity(64 * 1024);

    // ── head + header ──
    html.push_str(&format!(
r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>群聊分析报告</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
    font-family: 'Noto Sans SC', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    background: #5BCEFA; padding: 30px; line-height: 1.6; color: #2D3748;
}}
.container {{ max-width: 1200px; margin: 0 auto; background: #FFF; border-radius: 24px;
    box-shadow: 0 8px 40px rgba(245,169,184,0.12); overflow: hidden; display: flex; flex-direction: column; }}
.content {{ flex: 1; padding: 40px 45px; border-radius: 0 0 24px 24px; }}
.header {{ background: #5BCEFA; color: #fff; padding: 50px 50px 45px; text-align: center; }}
.header h1 {{ font-size: 2.8em; font-weight: 300; margin-bottom: 8px; letter-spacing: -0.5px; }}
.header .subtitle {{ font-size: 1.1em; opacity: 0.9; font-weight: 300; }}
.section {{ margin-bottom: 40px; }}
.section-title {{ font-size: 1.5em; font-weight: 600; margin-bottom: 22px; color: #4A5568;
    border-bottom: 2px solid #F5E0E8; padding-bottom: 10px; display: flex; align-items: center; gap: 10px; }}
.stats-grid {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 20px; margin-bottom: 30px; }}
.stat-card {{ background: #FFF5F8; padding: 30px 20px; text-align: center; border-radius: 16px; border: 1px solid #F5E0E8; }}
.stat-number {{ font-size: 2.8em; font-weight: 300; color: #5BCEFA; margin-bottom: 6px; letter-spacing: -1px; }}
.stat-label {{ font-size: 0.9em; color: #6B7280; text-transform: uppercase; letter-spacing: 1px; }}
.highlights-grid {{ display: grid; grid-template-columns: 1fr 1.6fr; gap: 20px; margin: 30px 0; }}
.active-period {{ background: #F5A9B8; color: #fff; padding: 28px 24px; border-radius: 18px; }}
.emoji-bar {{ background: #FFF5F8; padding: 22px 24px; border-radius: 18px; border: 1px solid #F5E0E8; }}
.leaderboard {{ background: #FFF5F8; padding: 28px 24px; border-radius: 18px; border: 1px solid #F5E0E8; }}
.lb-item {{ display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }}
.lb-avatar {{ width: 38px; height: 38px; border-radius: 50%; object-fit: cover; }}
.lb-bar {{ flex: 1; height: 8px; background: #F0F0F0; border-radius: 4px; overflow: hidden; }}
.lb-fill {{ height: 100%; border-radius: 4px; }}
.hourly-chart {{ display: flex; gap: 4px; align-items: flex-end; height: 140px; margin-top: 12px; padding: 8px 0; }}
.hourly-bar {{ flex: 1; border-radius: 4px 4px 0 0; position: relative; transition: all 0.3s; }}
.hourly-label {{ position: absolute; bottom: -20px; left: 50%; transform: translateX(-50%); font-size: 10px; color: #6B7280; }}
.topic-grid {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; }}
.topic-card {{ background: #FFF5F8; border-radius: 16px; padding: 22px; border: 1px solid #F5E0E8; }}
.topic-tag {{ display: inline-block; background: #5BCEFA; color: white; padding: 3px 10px; border-radius: 10px; font-size: 0.8em; margin-right: 4px; }}
.title-grid {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; }}
.title-card {{ background: #FFF5F8; border-radius: 16px; padding: 22px; border: 1px solid #F5E0E8; display: flex; gap: 14px; }}
.title-avatar {{ width: 56px; height: 56px; border-radius: 50%; object-fit: cover; flex-shrink: 0; }}
.mbti-badge {{ display: inline-block; background: #F5A9B8; color: white; padding: 2px 8px; border-radius: 8px; font-size: 0.75em; }}
.quote-item {{ background: #FFF5F8; border-left: 4px solid #F5A9B8; padding: 18px 20px; border-radius: 0 14px 14px 0; margin-bottom: 12px; }}
.rel-grid {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 16px; }}
.rel-card {{ background: #FFF5F8; border-radius: 16px; padding: 22px; border: 1px solid #F5E0E8; }}
.rel-label {{ display: inline-block; background: #5BCEFA; color: white; padding: 4px 12px; border-radius: 12px; font-size: 0.85em; margin-bottom: 8px; }}
.footer {{ background: #5BCEFA; color: #fff; padding: 20px; text-align: center; font-size: 0.85em; opacity: 0.9; }}
</style>
</head>
<body>
<div class="container">
<div class="header">
  <h1>{group_name} 群聊周报</h1>
  <div class="subtitle">{date_str}</div>
</div>
<div class="content">
"#));

    // ── 4-card stats ──
    html.push_str(r#"<div class="stats-grid">"#);
    for (num, label) in [
        (stats.message_count.to_string(), "消息总数"),
        (stats.participant_count.to_string(), "参与人数"),
        (stats.total_characters.to_string(), "总字数"),
        (stats.emoji_count.to_string(), "表情数量"),
    ] {
        html.push_str(&format!(
            r#"<div class="stat-card"><div class="stat-number">{num}</div><div class="stat-label">{label}</div></div>"#
        ));
    }
    html.push_str("</div>");

    // ── Highlights ──
    html.push_str(r#"<div class="highlights-grid"><div class="highlights-left">"#);
    html.push_str(&format!(
        r#"<div class="active-period"><div style="font-size:1.3em;font-weight:600;">🕐 最活跃时段</div><div style="font-size:1.8em;margin-top:8px;">{}</div></div>"#,
        html_escape(&stats.most_active_hour)
    ));
    // top faces
    html.push_str(r#"<div class="emoji-bar"><div style="font-weight:600;margin-bottom:8px;">🎭 热门表情 Top 3</div>"#);
    for (fid, cnt) in &stats.top_faces {
        html.push_str(&format!(r#"<span style="margin-right:12px;">[face:{fid}] ×{cnt}</span>"#));
    }
    html.push_str("</div></div>");
    // leaderboard
    html.push_str(r#"<div class="leaderboard"><div style="font-weight:600;margin-bottom:12px;">🏆 发言排行榜</div>"#);
    let max_count = stats.top_speakers.first().map(|s| s.2).unwrap_or(1);
    for (i, (uid, name, cnt)) in stats.top_speakers.iter().take(10).enumerate() {
        let pct = (*cnt as f64 / max_count as f64 * 100.0) as u32;
        let colors = ["#5BCEFA", "#F5A9B8", "#FFD700", "#90EE90", "#DDA0DD",
                       "#87CEEB", "#FFA07A", "#98FB98", "#DEB887", "#B0C4DE"];
        let color = colors[i % colors.len()];
        html.push_str(&format!(
            r#"<div class="lb-item"><img class="lb-avatar" src="https://q1.qlogo.cn/g?b=qq&nk={uid}&s=40" alt=""><div style="flex:1;"><div style="display:flex;justify-content:space-between;"><span>{name}</span><span style="color:#6B7280;">{cnt}条</span></div><div class="lb-bar"><div class="lb-fill" style="width:{pct}%;background:{color};"></div></div></div></div>"#,
            name = html_escape(name),
        ));
    }
    html.push_str("</div></div>");

    // ── Hourly chart ──
    html.push_str(r#"<div class="section"><div class="section-title">📊 24小时消息分布</div><div class="hourly-chart">"#);
    let h_max = *stats.hourly_distribution.iter().max().unwrap_or(&1).max(&1);
    for (h, &cnt) in stats.hourly_distribution.iter().enumerate() {
        let pct = (cnt as f64 / h_max as f64 * 100.0) as u32;
        let color = if cnt == h_max { "#F5A9B8" } else { "#5BCEFA" };
        html.push_str(&format!(
            r#"<div class="hourly-bar" style="height:{pct}%;background:{color};min-height:2px;"><span class="hourly-label">{h}</span></div>"#
        ));
    }
    html.push_str("</div></div>");

    // ── Topics ──
    if !llm.topics.is_empty() {
        html.push_str(r#"<div class="section"><div class="section-title">💬 话题盘点</div><div class="topic-grid">"#);
        for t in &llm.topics {
            html.push_str(r#"<div class="topic-card">"#);
            html.push_str(&format!(r#"<div style="font-weight:600;font-size:1.1em;margin-bottom:8px;">{}</div>"#, html_escape(&t.topic)));
            for c in &t.contributors {
                html.push_str(&format!(r#"<span class="topic-tag">{}</span>"#, html_escape(c)));
            }
            html.push_str(&format!(r#"<div style="margin-top:10px;color:#4A5568;font-size:0.9em;">{}</div>"#, html_escape(&t.detail)));
            html.push_str("</div>");
        }
        html.push_str("</div></div>");
    }

    // ── Titles ──
    if !llm.user_titles.is_empty() {
        html.push_str(r#"<div class="section"><div class="section-title">🎖️ 群友称号</div><div class="title-grid">"#);
        for ut in &llm.user_titles {
            let uid = name_to_uid.get(&ut.name).copied().unwrap_or(0);
            html.push_str(&format!(
                r#"<div class="title-card"><img class="title-avatar" src="https://q1.qlogo.cn/g?b=qq&nk={uid}&s=100" alt=""><div><div style="font-weight:600;">{name}</div><div style="font-size:1.2em;color:#F5A9B8;">{title}</div><span class="mbti-badge">{mbti}</span><div style="margin-top:6px;font-size:0.85em;color:#6B7280;">{habit}</div><div style="font-size:0.8em;color:#9CA3AF;">{reason}</div></div></div>"#,
                name = html_escape(&ut.name), title = html_escape(&ut.title),
                mbti = html_escape(&ut.mbti), habit = html_escape(&ut.habit),
                reason = html_escape(&ut.reason),
            ));
        }
        html.push_str("</div></div>");
    }

    // ── Quotes ──
    if !llm.golden_quotes.is_empty() {
        html.push_str(r#"<div class="section"><div class="section-title">📜 群圣经</div>"#);
        for q in &llm.golden_quotes {
            html.push_str(&format!(
                r#"<div class="quote-item"><div style="font-size:1.1em;">"{}"</div><div style="margin-top:6px;color:#F5A9B8;font-weight:600;">—— {}</div><div style="margin-top:4px;color:#6B7280;font-size:0.85em;">{}</div></div>"#,
                html_escape(&q.content), html_escape(&q.sender), html_escape(&q.reason),
            ));
        }
        html.push_str("</div>");
    }

    // ── Relationships ──
    if !llm.relationships.is_empty() {
        html.push_str(r#"<div class="section"><div class="section-title">🤝 关系图谱</div><div class="rel-grid">"#);
        for r in &llm.relationships {
            html.push_str(r#"<div class="rel-card">"#);
            html.push_str(&format!(r#"<div class="rel-label">{}</div>"#, html_escape(&r.label)));
            let members_str: Vec<String> = r.members.iter().map(|m| html_escape(m)).collect();
            html.push_str(&format!(r#"<div style="font-weight:600;margin:6px 0;">{}</div>"#, members_str.join(" × ")));
            html.push_str(&format!(r#"<div style="color:#4A5568;font-size:0.9em;">{}</div>"#, html_escape(&r.vibe)));
            if !r.evidence.is_empty() {
                html.push_str(r#"<div style="margin-top:8px;padding-top:8px;border-top:1px solid #F5E0E8;font-size:0.8em;color:#6B7280;">"#);
                for ev in &r.evidence {
                    html.push_str(&format!(r#"<div>「{}」</div>"#, html_escape(ev)));
                }
                html.push_str("</div>");
            }
            html.push_str("</div>");
        }
        html.push_str("</div></div>");
    }

    // ── Footer ──
    html.push_str(&format!(
        r#"</div><div class="footer">由 LianBot 生成 · {datetime_str} · Powered by DeepSeek</div></div></body></html>"#
    ));

    html
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ── P8: Chrome 无头截图（与 LianBot capture_sync 一致）──────────────────────

fn find_chrome() -> Option<String> {
    for name in &["google-chrome-stable", "google-chrome", "chromium-browser", "chromium"] {
        if std::process::Command::new("which").arg(name)
            .output().map(|o| o.status.success()).unwrap_or(false)
        { return Some(name.to_string()); }
    }
    None
}

fn capture_screenshot(html: &str, width: u32) -> Result<(String, Duration), String> {
    let chrome = find_chrome().ok_or("未找到 Chrome/Chromium")?;
    let start = Instant::now();

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {e}"))?;
    let tmp = tmp_dir.path();
    let html_path = tmp.join("page.html");
    let img_path = tmp.join("shot.png");
    let user_data = tmp.join("chrome-profile");
    std::fs::create_dir_all(&user_data).map_err(|e| format!("创建 profile 目录失败: {e}"))?;

    let user_data_arg = format!("--user-data-dir={}", user_data.display());
    let crash_dir_arg = format!("--crash-dumps-dir={}", tmp.join("crashes").display());

    // 注入高度测量 JS（与 LianBot 一致）
    let patched = html.replace(
        "</body>",
        "<script>(function(){const de=document.documentElement;const b=document.body;\
         const scrollH=Math.max(de?de.scrollHeight:0,de?de.offsetHeight:0,de?de.clientHeight:0,\
         b?b.scrollHeight:0,b?b.offsetHeight:0,b?b.clientHeight:0);\
         let mb=0;if(b){const m=document.createElement('div');m.style.cssText='display:block;height:1px;width:1px;';\
         b.appendChild(m);mb=m.getBoundingClientRect().bottom+(window.scrollY||window.pageYOffset||0);\
         const bs=window.getComputedStyle(b);mb+=parseFloat(bs.paddingBottom)||0;}\
         const h=Math.ceil(Math.max(scrollH,mb));document.title=String(h);})();</script></body>"
    );
    std::fs::write(&html_path, &patched).map_err(|e| format!("写 HTML 失败: {e}"))?;

    let base_args: &[&str] = &[
        "--headless=new", "--no-sandbox", "--disable-gpu",
        "--disable-dev-shm-usage", "--disable-breakpad",
    ];

    // 第一步: dump-dom 测高
    let dom_out = run_chrome(&chrome, &[
        &user_data_arg, &crash_dir_arg,
        "--hide-scrollbars", "--virtual-time-budget=3000", "--dump-dom",
        &format!("--window-size={width},2000"),
        &format!("file://{}", html_path.display()),
    ], base_args, 30)?;

    let dom_str = String::from_utf8_lossy(&dom_out);
    let height = extract_title_height(&dom_str).unwrap_or(4000);
    let height = (height + 24).clamp(600, 20000);

    // 第二步: 截图
    run_chrome(&chrome, &[
        &user_data_arg, &crash_dir_arg,
        "--hide-scrollbars", "--run-all-compositor-stages-before-draw",
        "--virtual-time-budget=5000",
        &format!("--screenshot={}", img_path.display()),
        &format!("--window-size={width},{height}"),
        &format!("file://{}", html_path.display()),
    ], base_args, 30)?;

    let img_data = std::fs::read(&img_path).map_err(|e| format!("读截图失败: {e}"))?;
    let elapsed = start.elapsed();
    Ok((B64.encode(&img_data), elapsed))
}

fn run_chrome(chrome: &str, extra_args: &[&str], base_args: &[&str], timeout_secs: u64) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new(chrome);
    for a in base_args { cmd.arg(a); }
    for a in extra_args { cmd.arg(a); }

    let mut child = cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Chrome 启动失败: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let out = child.wait_with_output().map_err(|e| format!("读输出失败: {e}"))?;
                return Ok(out.stdout);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Chrome 超时 ({timeout_secs}s)"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("Chrome 等待失败: {e}")),
        }
    }
}

fn extract_title_height(dom: &str) -> Option<u32> {
    let start = dom.find("<title>")? + 7;
    let end = dom[start..].find("</title>")? + start;
    dom[start..end].trim().parse().ok()
}

// ── Prompt 构造（与 LianBot 一致） ───────────────────────────────────────────

fn build_topics_prompt(text: &str) -> String {
    format!(
r#"你是一个群聊信息总结助手。请分析以下群聊记录，提取出讨论最热烈、信息量最大的主要话题，最多 8 个。

要求：
- 每个话题包含：话题名称、主要参与者（最多5人）、详细描述（包含谁发起、谁参与、讨论内容、是否有结论）
- 描述中提及群友时，必须使用 @名字 格式

群聊记录：
{text}

返回 JSON 数组：[{{"topic":"","contributors":[],"detail":""}}]"#)
}

fn build_titles_prompt(text: &str) -> String {
    format!(
r#"你是一个有趣的群聊分析师。请为最活跃的群友（最多6人）生成专属称号。

每人提供：昵称、称号（4-8字）、MBTI、发言习惯、理由
同时挑选最多5条"群圣经"（令人印象深刻的发言）

群聊记录：
{text}

返回 JSON：{{"user_titles":[{{"name":"","title":"","mbti":"","habit":"","reason":""}}],"golden_quotes":[{{"content":"","sender":"","reason":""}}]}}"#)
}

fn build_relationships_prompt(text: &str) -> String {
    format!(
r#"你是一个善于解读人际关系的分析师。请分析以下群聊记录中 2-6 组最有趣的关系。

每组提供：type（duo/group）、members、label（4-8字标签）、vibe（一句话点评）、evidence（1-3条原文佐证）

群聊记录：
{text}

返回 JSON 数组：[{{"type":"duo","members":[],"label":"","vibe":"","evidence":[]}}]"#)
}

// ── 示例文本（--llm-only 模式使用）─────────────────────────────────────────

fn sample_chat_text() -> &'static str {
r#"[09:00] 小雪: 早上好各位！今天有人一起打球吗
[09:01] 阿明: 下午可以，几点？
[09:02] 小雪: 3点老地方？
[09:03] 大佬: 我也去 这次一定要赢你
[09:05] 阿明: 大佬你上次打了个3:11 还立什么flag
[09:06] 大佬: 那是战术性放水 你不懂
[09:07] 小雪: 哈哈哈哈哈 战术性放水 我都笑了
[09:10] 路人甲: 你们打球我看看 我去当裁判
[09:15] 小白: 说到运动 昨天看了那个新出的动漫 主角也是打乒乓球的
[09:16] 阿明: 乒乓还是羽毛球啊 我们打的是羽毛球
[09:17] 小白: 哦哦 搞混了 反正都是球
[09:20] 程序猿: 各位大佬 有没有人用过Rust的tokio 我昨天写的异步代码死锁了
[09:21] 大佬: 看下有没有 await 忘写
[09:22] 程序猿: 确实有一个忘写了…谢谢大佬
[09:23] 阿明: @程序猿 你的代码跟你的球技一样 需要多练
[09:25] 路人甲: 哈哈哈 开始人身攻击了
[09:30] 小雪: 话说周末有人去那个新开的火锅店吗？听说巨好吃
[09:31] 大佬: 去去去 吃完正好消化一下
[09:32] 小白: 我也想去！在哪里
[09:33] 小雪: 就在地铁站旁边那个 叫什么来着…海底世界？
[09:34] 路人甲: 海底捞吧…海底世界是水族馆
[09:35] 小雪: 对对对 海底捞！脑子不好使了
[09:36] 阿明: 小雪你这记性 难怪球打不好
[09:37] 小雪: 你再说！下午加倍打你！
[10:00] 程序猿: 各位 我又有问题了 这个借用检查器也太严厉了
[10:01] 大佬: 贴代码看看
[10:02] 程序猿: fn process(data: &mut Vec<i32>) { let first = &data[0]; data.push(42); println!("{}", first); }
[10:03] 大佬: 经典同时持有可变和不可变引用 你先 clone 或者调整顺序
[10:04] 程序猿: 哦！谢谢 我还以为是bug
[10:05] 阿明: Rust编译器：这不是bug 这是feature
[10:06] 路人甲: 编译器："不是我不讲道理，是你不讲规矩"
[10:10] 小白: 你们程序员的世界好复杂
[10:11] 大佬: 比打球简单多了 至少编译器会告诉你哪里错了
[10:12] 阿明: 球也可以告诉你 球飞出界了说明你力量控制有问题
[10:15] 小雪: 行了行了 下午3点不见不散！"#
}

/// 将 --llm-only 模式的硬编码文本转为 ChatMessage 数组（用于后续统计/渲染）
fn sample_to_chat_messages() -> Vec<ChatMessage> {
    let text = sample_chat_text();
    let mut msgs = Vec::new();
    let base_ts = chrono::Utc::now().timestamp() - 3600 * 2;
    for (i, line) in text.lines().enumerate() {
        // [HH:MM] name: content
        let rest = line.strip_prefix('[').and_then(|s| {
            let end = s.find(']')?;
            Some((&s[..end], &s[end+1..]))
        });
        if let Some((_time, rest)) = rest {
            let rest = rest.trim_start();
            if let Some((name, content)) = rest.split_once(": ") {
                msgs.push(ChatMessage {
                    user_id: (i as i64) % 6 + 10001,
                    nickname: name.to_string(),
                    time: base_ts + i as i64 * 60,
                    text: content.to_string(),
                    emoji_count: 0, image_count: 0, reply_to: None,
                    at_targets: Vec::new(), face_ids: Vec::new(),
                });
            }
        }
    }
    msgs
}

// ── 阶段计时记录 ─────────────────────────────────────────────────────────────

struct PhaseTime {
    name: &'static str,
    ms: u64,
    detail: String,
}

// ── main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let root = find_project_root();
    let pipeline_start = Instant::now();
    let mut phases: Vec<PhaseTime> = Vec::new();

    println!();
    println!("═══════════════════════════════════════════════════════════════");
    println!("  SMY Pipeline Benchmark — LianBot (Full End-to-End)");
    println!("═══════════════════════════════════════════════════════════════");
    println!("  RSS at start: {:.1} MB", rss_mb());
    println!();

    let runtime: RuntimeToml = load_toml(&root.join("runtime.toml"));
    let logic: LogicToml = load_toml(&root.join("logic.toml"));
    let screenshot_width = logic.smy.screenshot_width;

    let llm_cfg = match logic.smy.llm {
        Some(mut cfg) => {
            if let Some(ref m) = args.model_override { cfg.model = m.clone(); }
            Some(cfg)
        }
        None => {
            eprintln!("⚠ logic.toml 中未配置 [smy.llm]，LLM 阶段将跳过");
            None
        }
    };

    println!("  NapCat:      {}", runtime.napcat.url);
    if let Some(ref cfg) = llm_cfg {
        println!("  LLM:        {} (model: {})", cfg.api_url, cfg.model);
    }
    println!("  截图宽度:    {}px", screenshot_width);
    println!("  时间范围:    最近 {} 天", args.days);
    println!("  LLM 轮数:   {}", args.rounds);
    println!("  截图:        {}", if args.no_screenshot { "跳过" } else { "启用" });
    println!();

    let client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("构建 HTTP 客户端失败");

    // ═════════════════════════════════════════════════════════════════════════
    // P1: NapCat 历史消息获取
    // ═════════════════════════════════════════════════════════════════════════

    let raw_messages: Vec<serde_json::Value>;
    let chat_messages: Vec<ChatMessage>;

    if args.llm_only {
        println!("━━━ P1: NapCat 历史获取 [跳过 --llm-only] ━━━");
        phases.push(PhaseTime { name: "P1 NapCat", ms: 0, detail: "跳过".into() });
        println!();

        raw_messages = Vec::new();
        chat_messages = sample_to_chat_messages();

        phases.push(PhaseTime { name: "P2 解析", ms: 0, detail: "跳过(示例数据)".into() });
    } else {
        let group_id = args.group_id
            .or_else(|| runtime.bot.initial_groups.first().copied())
            .unwrap_or_else(|| {
                eprintln!("✗ 请指定群号: --group <ID>（或在 runtime.toml 中配置 initial_groups）");
                std::process::exit(1);
            });

        let cutoff = chrono::Utc::now().timestamp() - (args.days as i64 * 86400);

        println!("━━━ P1: NapCat 历史获取 ━━━");
        println!("  群号: {group_id}  范围: 最近 {} 天", args.days);
        println!();

        match fetch_history(&client, &runtime.napcat, group_id, cutoff).await {
            Ok(result) => {
                let in_range = result.messages.iter()
                    .filter(|m| m["time"].as_i64().unwrap_or(0) >= cutoff)
                    .count();

                println!();
                println!("  ┌──────────────────────────────────────────┐");
                println!("  │ 总消息:  {:<10} 范围内: {:<10}    │", result.messages.len(), in_range);
                println!("  │ 分页数:  {:<10} 原始:   {:<10}    │", result.pages, human_size(result.raw_bytes));
                println!("  │ 耗时:    {:<10}                        │", format!("{}ms", result.total_fetch_time.as_millis()));
                println!("  └──────────────────────────────────────────┘");

                phases.push(PhaseTime {
                    name: "P1 NapCat",
                    ms: result.total_fetch_time.as_millis() as u64,
                    detail: format!("{}页 {}条 {}", result.pages, result.messages.len(), human_size(result.raw_bytes)),
                });

                let rss_after = rss_mb();
                println!("  RSS after fetch: {rss_after:.1} MB");
                println!();

                // ═══════════════════════════════════════════════════════════
                // P2: JSON → ChatMessage 解析
                // ═══════════════════════════════════════════════════════════

                println!("━━━ P2: JSON → ChatMessage 解析 ━━━");
                let t2 = Instant::now();
                let parsed = parse_raw_messages(&result.messages, Some(cutoff));
                let p2_ms = t2.elapsed().as_millis() as u64;

                println!("  输入: {} 条原始 JSON", result.messages.len());
                println!("  输出: {} 条 ChatMessage（过滤空消息/bot消息/cutoff）", parsed.len());
                println!("  耗时: {p2_ms}ms");
                println!("  RSS: {:.1} MB", rss_mb());

                phases.push(PhaseTime {
                    name: "P2 解析",
                    ms: p2_ms,
                    detail: format!("{}→{} msgs", result.messages.len(), parsed.len()),
                });
                println!();

                raw_messages = result.messages;
                chat_messages = parsed;
            }
            Err(e) => {
                eprintln!("  ✗ NapCat 获取失败: {e}");
                eprintln!("  回退到示例数据");
                phases.push(PhaseTime { name: "P1 NapCat", ms: 0, detail: format!("失败: {e}") });
                phases.push(PhaseTime { name: "P2 解析", ms: 0, detail: "回退".into() });
                raw_messages = Vec::new();
                chat_messages = sample_to_chat_messages();
            }
        }
    }
    let _ = &raw_messages; // suppress unused warning

    // ═════════════════════════════════════════════════════════════════════════
    // P3: Statistics 统计分析
    // ═════════════════════════════════════════════════════════════════════════

    println!("━━━ P3: Statistics 统计分析 ━━━");
    let t3 = Instant::now();
    let stats = analyze(&chat_messages);
    let p3_ms = t3.elapsed().as_millis() as u64;

    println!("  消息: {}  参与者: {}  字符: {}  表情: {}",
        stats.message_count, stats.participant_count, stats.total_characters, stats.emoji_count);
    println!("  最活跃: {}  回复: {}  @: {}",
        stats.most_active_hour, stats.reply_count, stats.at_count);
    println!("  Top 发言:");
    for (i, (_, name, cnt)) in stats.top_speakers.iter().take(3).enumerate() {
        println!("    {}. {} — {} 条", i + 1, name, cnt);
    }
    println!("  耗时: {p3_ms}ms");

    phases.push(PhaseTime {
        name: "P3 统计",
        ms: p3_ms,
        detail: format!("{} msgs → {} participants", stats.message_count, stats.participant_count),
    });
    println!();

    // ═════════════════════════════════════════════════════════════════════════
    // P4: format_for_llm 文本化
    // ═════════════════════════════════════════════════════════════════════════

    println!("━━━ P4: format_for_llm 文本化 ━━━");
    let t4 = Instant::now();
    let llm_text = format_for_llm(&chat_messages);
    let p4_ms = t4.elapsed().as_millis() as u64;

    let line_count = llm_text.lines().count();
    println!("  输出: {} ({} 行)", human_size(llm_text.len()), line_count);

    // 截断保护（与 LianBot 一致：80KB）
    let truncated = if llm_text.len() > 80_000 {
        println!("  ⚠ 超 80KB，截断为最后 80KB");
        &llm_text[llm_text.len() - 80_000..]
    } else { &llm_text };

    println!("  LLM 输入: {}", human_size(truncated.len()));
    println!("  耗时: {p4_ms}ms");

    phases.push(PhaseTime {
        name: "P4 格式化",
        ms: p4_ms,
        detail: format!("{} lines → {}", line_count, human_size(truncated.len())),
    });
    println!();

    // ═════════════════════════════════════════════════════════════════════════
    // P5: LLM 3× 并行调用
    // ═════════════════════════════════════════════════════════════════════════

    let mut llm_result = LlmResult::default();
    let mut all_llm_results: Vec<Vec<LlmBenchResult>> = Vec::new();

    if let Some(ref llm_cfg) = llm_cfg {
        if truncated.len() < 50 {
            println!("━━━ P5: LLM 分析 [跳过: 文本 < 50 字节] ━━━");
            phases.push(PhaseTime { name: "P5 LLM", ms: 0, detail: "文本太短,跳过".into() });
        } else {
            println!("━━━ P5: LLM 3× 并行分析 ━━━");
            println!("  模型: {}  输入: {}", llm_cfg.model, human_size(truncated.len()));
            println!();

            let prompts = [
                ("topics", build_topics_prompt(truncated)),
                ("titles", build_titles_prompt(truncated)),
                ("relationships", build_relationships_prompt(truncated)),
            ];

            for round in 0..args.rounds {
                if args.rounds > 1 {
                    println!("  ── 第 {} / {} 轮 ──", round + 1, args.rounds);
                }

                let t5 = Instant::now();
                let (r1, r2, r3) = tokio::join!(
                    bench_llm_call(&client, llm_cfg, "topics", &prompts[0].1),
                    bench_llm_call(&client, llm_cfg, "titles", &prompts[1].1),
                    bench_llm_call(&client, llm_cfg, "relationships", &prompts[2].1),
                );

                let round_results = vec![r1, r2, r3];
                let parallel_ms = round_results.iter().map(|r| r.total_ms).max().unwrap_or(0);
                let serial_ms: u64 = round_results.iter().map(|r| r.total_ms).sum();
                let wall_ms = t5.elapsed().as_millis() as u64;

                println!();
                println!("    并行耗时: {parallel_ms}ms  (串行: {serial_ms}ms  节省: {}ms  wall: {wall_ms}ms)",
                    serial_ms.saturating_sub(parallel_ms));
                println!("    RSS: {:.1} MB", rss_mb());

                // 最后一轮用于后续阶段
                if round == args.rounds - 1 {
                    phases.push(PhaseTime {
                        name: "P5 LLM",
                        ms: wall_ms,
                        detail: format!("3×并行 wall={wall_ms}ms max={parallel_ms}ms"),
                    });
                }

                all_llm_results.push(round_results);
                println!();
            }

            // ═════════════════════════════════════════════════════════════
            // P6: LLM JSON 清洗 + 反序列化
            // ═════════════════════════════════════════════════════════════

            println!("━━━ P6: LLM 响应 JSON 解析 ━━━");
            let t6 = Instant::now();

            if let Some(last_round) = all_llm_results.last() {
                // topics
                if last_round[0].success {
                    llm_result.topics = parse_llm_topics(&last_round[0].content);
                    println!("  topics:        {} 个话题  (content: {} chars)", llm_result.topics.len(), last_round[0].content.len());
                }
                // titles + quotes
                if last_round[1].success {
                    let (titles, quotes) = parse_llm_titles(&last_round[1].content);
                    println!("  titles:        {} 个称号, {} 条圣经  (content: {} chars)", titles.len(), quotes.len(), last_round[1].content.len());
                    llm_result.user_titles = titles;
                    llm_result.golden_quotes = quotes;
                }
                // relationships
                if last_round[2].success {
                    llm_result.relationships = parse_llm_relationships(&last_round[2].content);
                    println!("  relationships: {} 组关系  (content: {} chars)", llm_result.relationships.len(), last_round[2].content.len());
                }
            }

            let p6_ms = t6.elapsed().as_millis() as u64;
            println!("  耗时: {p6_ms}ms");

            phases.push(PhaseTime {
                name: "P6 JSON解析",
                ms: p6_ms,
                detail: format!("t{}+u{}+q{}+r{}",
                    llm_result.topics.len(), llm_result.user_titles.len(),
                    llm_result.golden_quotes.len(), llm_result.relationships.len()),
            });
            println!();
        }
    } else {
        println!("━━━ P5: LLM 分析 [跳过: 未配置] ━━━");
        phases.push(PhaseTime { name: "P5 LLM", ms: 0, detail: "未配置".into() });
        phases.push(PhaseTime { name: "P6 JSON解析", ms: 0, detail: "跳过".into() });
        println!();
    }

    // ═════════════════════════════════════════════════════════════════════════
    // P7: HTML 渲染
    // ═════════════════════════════════════════════════════════════════════════

    println!("━━━ P7: HTML 渲染 ━━━");
    let t7 = Instant::now();
    let html = render_html(&stats, &llm_result, "Benchmark 测试群", &chat_messages);
    let p7_ms = t7.elapsed().as_millis() as u64;

    println!("  HTML 大小: {} ({} 行)", human_size(html.len()), html.lines().count());
    println!("  耗时: {p7_ms}ms");
    println!("  RSS: {:.1} MB", rss_mb());

    phases.push(PhaseTime {
        name: "P7 渲染",
        ms: p7_ms,
        detail: format!("{} HTML", human_size(html.len())),
    });
    println!();

    // ═════════════════════════════════════════════════════════════════════════
    // P8: Chrome 无头截图
    // ═════════════════════════════════════════════════════════════════════════

    if args.no_screenshot {
        println!("━━━ P8: Chrome 截图 [跳过 --no-screenshot] ━━━");
        phases.push(PhaseTime { name: "P8 截图", ms: 0, detail: "跳过".into() });
    } else if find_chrome().is_none() {
        println!("━━━ P8: Chrome 截图 [跳过: 未安装 Chrome/Chromium] ━━━");
        eprintln!("  ⚠ 安装: apt install google-chrome-stable");
        phases.push(PhaseTime { name: "P8 截图", ms: 0, detail: "无Chrome".into() });
    } else {
        println!("━━━ P8: Chrome 无头截图 ━━━");
        println!("  Chrome: {}", find_chrome().unwrap());
        println!("  宽度: {screenshot_width}px");

        // spawn_blocking 模拟真实路径
        let html_clone = html.clone();
        let t8 = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            capture_screenshot(&html_clone, screenshot_width)
        }).await;

        match result {
            Ok(Ok((b64, chrome_elapsed))) => {
                let p8_ms = t8.elapsed().as_millis() as u64;
                let img_bytes = B64.decode(&b64).unwrap_or_default();
                println!("  PNG: {} (base64: {})", human_size(img_bytes.len()), human_size(b64.len()));
                println!("  Chrome 内部: {}ms  总计: {p8_ms}ms", chrome_elapsed.as_millis());
                println!("  RSS: {:.1} MB", rss_mb());

                phases.push(PhaseTime {
                    name: "P8 截图",
                    ms: p8_ms,
                    detail: format!("{} PNG", human_size(img_bytes.len())),
                });
            }
            Ok(Err(e)) => {
                let p8_ms = t8.elapsed().as_millis() as u64;
                eprintln!("  ✗ 截图失败: {e}");
                phases.push(PhaseTime { name: "P8 截图", ms: p8_ms, detail: format!("失败: {e}") });
            }
            Err(e) => {
                eprintln!("  ✗ 截图任务 panic: {e}");
                phases.push(PhaseTime { name: "P8 截图", ms: 0, detail: "panic".into() });
            }
        }
    }

    println!();

    // ═════════════════════════════════════════════════════════════════════════
    // Summary
    // ═════════════════════════════════════════════════════════════════════════

    let total_ms = pipeline_start.elapsed().as_millis() as u64;

    println!("═══════════════════════════════════════════════════════════════");
    println!("  Summary — 全管线端到端基准");
    println!("═══════════════════════════════════════════════════════════════");
    println!();

    // 阶段耗时表
    println!("  阶段             耗时         占比    详情");
    println!("  ─────────────────────────────────────────────────────────");
    let max_phase_ms = phases.iter().map(|p| p.ms).max().unwrap_or(1);
    for p in &phases {
        let pct = if total_ms > 0 { p.ms as f64 / total_ms as f64 * 100.0 } else { 0.0 };
        println!("  {:<14} {:>7}ms  {:>5.1}%    {}", p.name, p.ms, pct, p.detail);
    }
    println!("  ─────────────────────────────────────────────────────────");
    println!("  {:<14} {:>7}ms  100.0%", "总计", total_ms);
    println!();

    // 条形图
    println!("  耗时分布：");
    for p in &phases {
        if p.ms > 0 {
            bar(p.name, p.ms, max_phase_ms);
        }
    }
    println!();

    // LLM 多轮统计
    if !all_llm_results.is_empty() {
        println!("  LLM 详细统计（{} 轮）：", all_llm_results.len());
        let prompt_names = ["topics", "titles", "relationships"];
        for (idx, name) in prompt_names.iter().enumerate() {
            let times: Vec<u64> = all_llm_results.iter().map(|r| r[idx].total_ms).collect();
            let avg = times.iter().sum::<u64>() / times.len().max(1) as u64;
            let min = times.iter().copied().min().unwrap_or(0);
            let max = times.iter().copied().max().unwrap_or(0);
            let conn_avg = all_llm_results.iter().map(|r| r[idx].connect_ms).sum::<u64>()
                / all_llm_results.len().max(1) as u64;
            println!("    {name:<16} avg={avg:>6}ms  min={min}ms  max={max}ms  conn={conn_avg}ms");
        }

        let parallel_times: Vec<u64> = all_llm_results.iter()
            .map(|r| r.iter().map(|x| x.total_ms).max().unwrap_or(0)).collect();
        let p_avg = parallel_times.iter().sum::<u64>() / parallel_times.len().max(1) as u64;
        println!("    {:<16} avg={:>6}ms", "parallel_max", p_avg);

        let total_calls = all_llm_results.iter().flat_map(|r| r.iter()).count();
        let ok_calls = all_llm_results.iter().flat_map(|r| r.iter()).filter(|r| r.success).count();
        println!("    成功率: {ok_calls}/{total_calls}");

        let errors: Vec<_> = all_llm_results.iter().flat_map(|r| r.iter())
            .filter(|r| !r.success).collect();
        if !errors.is_empty() {
            println!("    错误:");
            for e in &errors {
                println!("      ✗ {} — {}", e.name, e.error.as_deref().unwrap_or("未知"));
            }
        }
        println!();
    }

    // 资源
    println!("  最终 RSS: {:.1} MB", rss_mb());
    println!();
}
