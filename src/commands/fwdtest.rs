// ── 合并转发测试命令（临时，测完删除）───────────────────────────────────────────
//
// !!fwdtest parse  — 在 pool 中找最近一条 forward 消息，递归解析并回复摘要
// !!fwdtest send   — 构造两条合并转发发送：1) 代码讨论 2) 图片集（嵌套转发）

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::commands::{Command, CommandContext, CommandKind, http_client};
use crate::runtime::pool::{MessagePool, MsgKind};
use crate::runtime::typ::MessageSegment;

pub struct FwdTestCommand;

#[async_trait]
impl Command for FwdTestCommand {
    fn name(&self) -> &str { "fwdtest" }
    fn help(&self) -> &str { "合并转发测试（临时命令）\n· parse — 解析最近的转发消息\n· send — 发送测试转发消息" }
    fn kind(&self) -> CommandKind { CommandKind::Simple }
    fn accepts_trailing(&self) -> bool { true }

    async fn execute(&self, ctx: CommandContext) -> Result<()> {
        let sub = ctx.get(&["_args"]).unwrap_or("").to_string();
        match sub.as_str() {
            "parse" => do_parse(ctx).await,
            "send"  => do_send(ctx).await,
            _ => ctx.reply("用法：!!fwdtest parse | send").await,
        }
    }
}

// ── parse：找 pool 中最近的 forward 消息，递归展开 ────────────────────────────

async fn do_parse(ctx: CommandContext) -> Result<()> {
    let pool = match &ctx.pool {
        Some(p) => p.clone(),
        None => return ctx.reply("❌ 消息池不可用").await,
    };

    let scope = ctx.bot_user.scope;
    let recent = pool.recent_internal(&scope, 50).await;

    // 找最近一条 forward 类型消息
    let fwd_msg = recent.iter().rev().find(|m| m.kind == MsgKind::Forward);
    let fwd_msg = match fwd_msg {
        Some(m) => m,
        None => return ctx.reply("❌ 最近 50 条消息中没有找到合并转发消息").await,
    };

    // 从 segments 中提取 forward id
    let fwd_id: Option<String> = fwd_msg.segments.iter()
        .find_map(|s| s.forward_id().map(String::from));
    let fwd_id = match fwd_id {
        Some(id) => id,
        None => return ctx.reply("❌ forward 消息段中未找到 id").await,
    };

    ctx.reply(&format!("🔍 正在解析转发消息 id={fwd_id}...")).await?;

    match ctx.api.get_forward_msg(&fwd_id).await {
        Ok(nodes) => {
            let mut lines = vec![format!("✅ 解析成功，共 {} 个顶层节点：", nodes.len())];
            for (i, node) in nodes.iter().enumerate() {
                let text_preview: String = node.segments.iter()
                    .filter_map(|s| s.as_text())
                    .collect::<Vec<_>>()
                    .join("")
                    .chars()
                    .take(40)
                    .collect();
                let seg_types: Vec<&str> = node.segments.iter()
                    .map(|s| s.seg_type.as_str())
                    .collect();
                let nested_info = if node.nested.is_empty() {
                    String::new()
                } else {
                    format!(" [嵌套 {} 条]", node.nested.len())
                };
                lines.push(format!(
                    "  [{i}] {nick} | segs={types} | {preview}{nested}",
                    nick = node.nickname,
                    types = seg_types.join("+"),
                    preview = if text_preview.is_empty() { "(无文字)".into() } else { text_preview },
                    nested = nested_info,
                ));
            }
            ctx.reply(&lines.join("\n")).await
        }
        Err(e) => ctx.reply(&format!("❌ 解析失败：{e}")).await,
    }
}

// ── send：构造两条合并转发发送 ────────────────────────────────────────────────

async fn do_send(ctx: CommandContext) -> Result<()> {
    let bot_id = 3571275661_i64; // bot QQ

    ctx.reply("📤 正在构造合并转发消息...").await?;

    // ── 第一条：代码讨论（多 text 节点）───────────────────────────────────────

    let code_nodes = vec![
        MessageSegment::node(bot_id, "恋", vec![
            MessageSegment::text("大家好，我写了一个 Rust 递归解析合并转发的模块："),
        ]),
        MessageSegment::node(bot_id, "恋", vec![
            MessageSegment::text("```rust\npub async fn get_forward_msg(&self, id: &str) -> Result<Vec<ForwardNode>> {\n    self.get_forward_msg_inner(id, 0).await\n}\n```"),
        ]),
        MessageSegment::node(bot_id, "恋", vec![
            MessageSegment::text("关键点：递归展开嵌套转发，深度上限 5 层，防止无限递归。"),
        ]),
        MessageSegment::node(bot_id, "恋", vec![
            MessageSegment::text("用 Box::pin 解决 async fn 递归编译问题。"),
        ]),
    ];

    ctx.api.send_forward_msg(
        ctx.bot_user.scope.into(),
        code_nodes,
        Some("代码审查记录"),
        Some("查看 4 条代码讨论"),
        Some("[聊天记录]"),
    ).await?;

    info!("[fwdtest] 第一条合并转发（代码讨论）已发送");

    // ── 第二条：文字 + 嵌套图片合并转发 ───────────────────────────────────────

    // 先获取 3 张 acg 图片 URL
    let mut image_urls = Vec::new();
    for _ in 0..3 {
        match resolve_acg_url().await {
            Some(url) => image_urls.push(url),
            None => image_urls.push("https://www.loliapi.com/bg/".into()),
        }
    }

    // 构造内层转发的图片节点
    let image_nodes: Vec<MessageSegment> = image_urls.iter().enumerate().map(|(i, url)| {
        MessageSegment::node(bot_id, "恋", vec![
            MessageSegment::text(&format!("第 {} 张：", i + 1)),
            MessageSegment::image(url),
        ])
    }).collect();

    // 先发送内层转发拿到 resId —— NapCat 的 send_forward_msg 支持嵌套 node
    // 实际上 OneBot 的做法是直接在外层 node 的 content 里放 node 段
    // 所以外层构造：一个文字节点 + 图片节点们全部平铺在同一个转发里
    let mut outer_nodes = vec![
        MessageSegment::node(bot_id, "恋", vec![
            MessageSegment::text("很多二次元图片 🎨"),
        ]),
    ];
    // 把图片节点作为后续的 node 拼进去
    outer_nodes.extend(image_nodes);

    ctx.api.send_forward_msg(
        ctx.bot_user.scope.into(),
        outer_nodes,
        Some("二次元图片集"),
        Some("查看 4 条图片消息"),
        Some("[聊天记录]"),
    ).await?;

    info!("[fwdtest] 第二条合并转发（图片集）已发送");
    ctx.reply("✅ 两条合并转发消息已发送").await
}

/// 从 loliapi 获取落地图片 URL（跟随 302 重定向）
async fn resolve_acg_url() -> Option<String> {
    let resp = http_client().get("https://www.loliapi.com/bg/").send().await.ok()?;
    let url = resp.url().to_string();
    if url != "https://www.loliapi.com/bg/" { Some(url) } else { None }
}
