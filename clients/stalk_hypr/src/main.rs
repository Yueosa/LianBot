use std::{
    env,
    io::Cursor,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    process::Command,
    sync::mpsc,
    time::sleep,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

// ── 配置 ──────────────────────────────────────────────────────────────────────

fn server_uri() -> String {
    env::var("SERVER_URI").unwrap_or_else(|_| "ws://127.0.0.1:8080/wstalk".into())
}

fn private_ws() -> u32 {
    env::var("PRIVATE_WS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9)
}

const HEARTBEAT_SECS: u64 = 30;
const JPEG_QUALITY: u8 = 75;

/// 返回 ~/.config/stalk_hypr/.env 路径（若 HOME 可取得）
fn config_env_path() -> Option<std::path::PathBuf> {
    let home = env::var("HOME").ok()?;
    Some(std::path::Path::new(&home).join(".config/stalk_hypr/.env"))
}

// ── Wayland 环境自愈 ──────────────────────────────────────────────────────────
//
// 在 systemd 用户服务 / 自动启动场景下，WAYLAND_DISPLAY 和
// HYPRLAND_INSTANCE_SIGNATURE 经常缺失，需要从 XDG_RUNTIME_DIR 推断。

fn get_uid() -> u32 {
    // 从 /proc/self/status 读取 Uid 行，避免引入 libc
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(1000)
}

fn fix_wayland_env() {
    let runtime_dir = env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", get_uid()));

    // WAYLAND_DISPLAY
    let need_wd = env::var("WAYLAND_DISPLAY")
        .map(|v| v.is_empty() || v.contains("swww"))
        .unwrap_or(true);

    if need_wd {
        let mut sockets: Vec<String> = std::fs::read_dir(&runtime_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("wayland-")
                    && !name.ends_with(".lock")
                    && !name.contains("swww")
                {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        sockets.sort();

        let target = sockets
            .iter()
            .find(|s| s.split('-').last().map(|n| n.parse::<u32>().is_ok()).unwrap_or(false))
            .or_else(|| sockets.first())
            .cloned();

        if let Some(t) = target {
            info!("auto WAYLAND_DISPLAY={t}");
            unsafe { env::set_var("WAYLAND_DISPLAY", t) };
        }
    }

    // HYPRLAND_INSTANCE_SIGNATURE
    if env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
        let hypr_dir = format!("{runtime_dir}/hypr");
        if let Ok(entries) = std::fs::read_dir(&hypr_dir) {
            let sig = entries
                .flatten()
                .find(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned());
            if let Some(s) = sig {
                info!("auto HYPRLAND_INSTANCE_SIGNATURE={s}");
                unsafe { env::set_var("HYPRLAND_INSTANCE_SIGNATURE", s) };
            }
        }
    }
}

// ── 当前 Hyprland 工作区 ──────────────────────────────────────────────────────

async fn active_workspace() -> Option<u32> {
    let out = Command::new("hyprctl")
        .args(["activeworkspace", "-j"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v["id"].as_u64().map(|n| n as u32)
}

// ── 截图 ──────────────────────────────────────────────────────────────────────

async fn grim_capture() -> Option<Vec<u8>> {
    let out = Command::new("grim")
        .arg("-")
        .output()
        .await
        .map_err(|e| error!("grim 启动失败: {e}"))
        .ok()?;

    if !out.status.success() {
        error!("grim 返回错误: {}", String::from_utf8_lossy(&out.stderr));
        return None;
    }
    Some(out.stdout)
}

/// PNG bytes → JPEG bytes，在 blocking 线程执行避免阻塞 event loop
fn png_to_jpeg(png: Vec<u8>) -> Option<Vec<u8>> {
    let img = image::load_from_memory(&png)
        .map_err(|e| warn!("图片解码失败: {e}"))
        .ok()?;

    let rgb = img.to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, JPEG_QUALITY);
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ColorType::Rgb8.into(),
    )
    .map_err(|e| warn!("JPEG 编码失败: {e}"))
    .ok()?;
    Some(buf.into_inner())
}

async fn take_screenshot() -> Option<String> {
    let png = grim_capture().await?;
    let raw_kb = png.len() / 1024;

    let (mime, bytes) = tokio::task::spawn_blocking(move || match png_to_jpeg(png.clone()) {
        Some(j) => ("image/jpeg", j),
        None    => ("image/png", png),
    })
    .await
    .ok()?;

    info!("screenshot: {raw_kb}KB PNG → {}KB {mime}", bytes.len() / 1024);
    Some(format!("data:{mime};base64,{}", B64.encode(&bytes)))
}

// ── WebSocket 会话 ─────────────────────────────────────────────────────────────
//
// 架构：
//   mpsc channel  ──(outgoing msgs)──►  writer task  ──►  WebSocket sink
//   reader loop   ◄──(incoming msgs)──  WebSocket stream
//
// 这样 heartbeat task 和 reader loop 都可以通过 channel 发送消息，
// 无需 clone 不可 clone 的 SplitSink。

async fn run_session(uri: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (ws_stream, _) = connect_async(uri).await?;
    info!("已连接 {uri}");

    let (mut ws_sink, mut ws_src) = ws_stream.split();

    // outgoing channel：所有需要发消息的地方都 send 到此 channel
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(32);

    // writer task：从 channel 取消息写入 ws sink
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // heartbeat task
    let hb_tx = out_tx.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(HEARTBEAT_SECS)).await;
            if hb_tx.send(Message::Text("heartbeat".into())).await.is_err() {
                break;
            }
        }
    });

    // reader loop（主 task）
    while let Some(msg) = ws_src.next().await {
        let text = match msg? {
            Message::Text(t)     => t.to_string(),
            Message::Close(_)    => break,
            Message::Ping(data)  => {
                let _ = out_tx.send(Message::Pong(data)).await;
                continue;
            }
            _ => continue,
        };

        debug!("收到: {text}");

        let Some(group_id) = text.strip_prefix("stalk:") else { continue };
        let group_id = group_id.trim().to_string();

        // 隐私工作区保护
        if let Some(ws_id) = active_workspace().await {
            if ws_id == private_ws() {
                warn!("工作区 {ws_id} 为隐私区，拒绝截图");
                let _ = out_tx
                    .send(Message::Text(
                        format!("stalk_text:{group_id}:主人正在看私密的东西哦 🙈").into(),
                    ))
                    .await;
                continue;
            }
        }

        info!("为群 {group_id} 截图...");
        let reply = match take_screenshot().await {
            Some(img) => format!("stalk_result:{group_id}:{img}"),
            None      => format!("stalk_text:{group_id}:截图失败，请稍后再试 😢"),
        };
        let _ = out_tx.send(Message::Text(reply.into())).await;
        info!("已回复群 {group_id}");
    }

    heartbeat.abort();
    writer.abort();
    Ok(())
}

// ── 主入口 ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // 优先加载 ~/.config/stalk_hypr/.env，其次尝试当前目录 .env
    let config_env = config_env_path();
    if let Some(ref p) = config_env {
        dotenvy::from_path(p).ok();
    }
    dotenvy::dotenv().ok(); // 当前目录 .env（若存在）可覆盖

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // 检查 grim
    if which::which("grim").is_err() {
        error!("未找到 grim，请安装: sudo pacman -S grim");
        std::process::exit(1);
    }

    fix_wayland_env();

    let uri = server_uri();
    info!("目标服务器: {uri}");

    let mut delay = 5u64;
    loop {
        match run_session(&uri).await {
            Ok(())  => info!("连接正常断开"),
            Err(e)  => warn!("连接出错: {e}"),
        }
        info!("{delay} 秒后重连...");
        sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(60); // 指数退避，上限 60s
    }
}
