use std::collections::HashMap;

use super::fetcher::ChatMessage;
use super::llm::LlmResult;
use super::statistics::Statistics;

// ── HTML 报告渲染 ─────────────────────────────────────────────────────────────
//
// 标准 MTF 粉蓝白配色方案 (Transgender Flag):
//   淡蓝: #5BCEFA   粉色: #F5A9B8   白色: #FFFFFF
//   背景: #FFF9FB    文字: #2D3748    次文字: #6B7280
//   纯色为主，不使用蓝粉渐变

pub fn render(
    stats: &Statistics,
    llm: &LlmResult,
    group_name: &str,
    messages: &[ChatMessage],
) -> String {
    // 构建 nickname → user_id 映射（用于头像 URL）
    let mut name_to_uid: HashMap<String, i64> = HashMap::new();
    for msg in messages {
        name_to_uid.entry(msg.nickname.clone()).or_insert(msg.user_id);
    }

    let date = crate::runtime::time::now();
    let date_str = date.format("%Y年%m月%d日").to_string();
    let datetime_str = date.format("%Y-%m-%d %H:%M").to_string();

    let hourly_chart = render_hourly_chart(&stats.hourly_distribution);
    let topics_html = render_topics(&llm.topics);
    let titles_html = render_user_titles(&llm.user_titles, &name_to_uid);
    let quotes_html = render_quotes(&llm.golden_quotes);
    let relationships_html = render_relationships(&llm.relationships, &name_to_uid);
    let highlights_html = render_highlights(stats);

    format!(
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
    background: #5BCEFA;
    padding: 30px;
    line-height: 1.6;
    color: #2D3748;
}}
.container {{
    max-width: 1200px;
    margin: 0 auto;
    background: #FFFFFF;
    border-radius: 24px;
    box-shadow: 0 8px 40px rgba(245,169,184,0.12);
    overflow: visible;
    display: flex;
    flex-direction: column;
}}
.content {{
    padding: 40px 45px;
}}
.content > .section:last-child {{
    margin-bottom: 0;
}}
.header {{
    background: #5BCEFA;
    color: #fff;
    border-radius: 24px 24px 0 0;
    padding: 50px 50px 45px;
    text-align: center;
}}
.header h1 {{
    font-size: 2.8em;
    font-weight: 300;
    margin-bottom: 8px;
    letter-spacing: -0.5px;
}}
.header .subtitle {{
    font-size: 1.1em;
    opacity: 0.9;
    font-weight: 300;
}}

/* ── Content ── */
.section {{ margin-bottom: 40px; }}
.section-title {{
    font-size: 1.5em;
    font-weight: 600;
    margin-bottom: 22px;
    color: #4A5568;
    border-bottom: 2px solid #F5E0E8;
    padding-bottom: 10px;
    display: flex;
    align-items: center;
    gap: 10px;
}}

/* ── Stats Grid ── */
.stats-grid {{
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 20px;
    margin-bottom: 30px;
}}
.stat-card {{
    background: #FFF5F8;
    padding: 30px 20px;
    text-align: center;
    border-radius: 16px;
    border: 1px solid #F5E0E8;
    transition: all 0.3s;
}}
.stat-card:hover {{
    transform: translateY(-3px);
    box-shadow: 0 8px 24px rgba(245,169,184,0.15);
}}
.stat-number {{
    font-size: 2.8em;
    font-weight: 300;
    color: #5BCEFA;
    margin-bottom: 6px;
    letter-spacing: -1px;
}}
.stat-label {{
    font-size: 0.9em;
    color: #6B7280;
    text-transform: uppercase;
    letter-spacing: 1px;
}}

/* ── Highlights Grid ── */
.highlights-grid {{
    display: grid;
    grid-template-columns: 1fr 1.6fr;
    gap: 20px;
    margin: 30px 0;
    align-items: stretch;
}}
.highlights-left {{
    display: flex;
    flex-direction: column;
    gap: 16px;
}}
.active-period {{
    background: #F5A9B8;
    color: #fff;
    padding: 28px 24px;
    border-radius: 18px;
    box-shadow: 0 6px 20px rgba(245,169,184,0.2);
    flex: 1;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
}}
.active-period .time {{
    font-size: 2.2em;
    font-weight: 600;
    margin-bottom: 6px;
    letter-spacing: -0.5px;
}}
.active-period .label {{
    font-size: 0.95em;
    opacity: 0.85;
    letter-spacing: 1px;
    text-transform: uppercase;
    margin-bottom: 10px;
}}
.active-period-sub {{
    font-size: 0.82em;
    opacity: 0.75;
}}
.top-emoji-card {{
    background: #FFF5F8;
    border: 1px solid #F5E0E8;
    border-radius: 16px;
    padding: 20px 24px;
    text-align: center;
    flex-shrink: 0;
}}
.top-emoji-card .te-label {{
    font-size: 0.88em;
    color: #6B7280;
    text-transform: uppercase;
    letter-spacing: 1px;
    margin-bottom: 8px;
}}
.top-emoji-card .te-items {{
    display: flex;
    justify-content: center;
    gap: 18px;
}}
.top-emoji-card .te-item {{
    display: flex;
    align-items: center;
    gap: 4px;
}}
.top-emoji-card .te-face {{
    font-size: 2em;
    line-height: 1.2;
}}
.top-emoji-card .te-cnt {{
    font-size: 0.82em;
    color: #9CA3AF;
}}
/* ── Leaderboard ── */
.leaderboard {{
    background: #FFFAFB;
    border: 1px solid #F5E0E8;
    border-radius: 18px;
    padding: 24px;
    display: flex;
    flex-direction: column;
    height: 100%;
    box-sizing: border-box;
}}
.leaderboard-title {{
    font-size: 1.0em;
    font-weight: 600;
    color: #4A5568;
    margin-bottom: 18px;
    text-align: center;
    letter-spacing: 1px;
    text-transform: uppercase;
}}
.speaker-row {{
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 14px;
    padding: 10px 12px;
    background: #fff;
    border-radius: 12px;
    border: 1px solid #F5E0E8;
}}
.speaker-rank {{
    font-size: 1.4em;
    flex-shrink: 0;
    width: 28px;
    text-align: center;
}}
.speaker-avatar {{
    width: 38px;
    height: 38px;
    border-radius: 50%;
    background: #F5A9B8;
    flex-shrink: 0;
    object-fit: cover;
}}
.speaker-info {{
    flex: 1;
    min-width: 0;
}}
.speaker-name {{
    font-weight: 600;
    font-size: 0.92em;
    color: #2D3748;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-bottom: 5px;
}}
.speaker-bar-wrap {{
    background: #F5E0E8;
    height: 6px;
    border-radius: 3px;
    overflow: hidden;
}}
.speaker-bar {{
    background: #5BCEFA;
    height: 100%;
    border-radius: 3px;
}}
.speaker-count {{
    font-size: 0.85em;
    font-weight: 600;
    color: #5BCEFA;
    flex-shrink: 0;
}}

/* ── Hourly Chart ── */
.chart-container {{
    background: #FFFAFB;
    padding: 30px;
    border-radius: 18px;
    border: 1px solid #F5E0E8;
    margin-bottom: 10px;
}}
.chart-title {{
    font-size: 1.1em;
    font-weight: 600;
    color: #4A5568;
    margin-bottom: 18px;
}}
.hour-bar-container {{
    display: flex;
    align-items: center;
    margin: 4px 0;
    height: 22px;
}}
.hour-label {{
    width: 55px;
    font-size: 0.82em;
    color: #6B7280;
    text-align: right;
    padding-right: 12px;
    flex-shrink: 0;
}}
.bar-wrapper {{
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
}}
.bar {{
    height: 16px;
    background: #5BCEFA;
    border-radius: 8px;
    min-width: 2px;
    transition: width 0.3s;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: 6px;
}}
.bar-value {{
    font-size: 0.72em;
    color: #fff;
    font-weight: 500;
    white-space: nowrap;
}}
.bar-value-outside {{
    font-size: 0.72em;
    color: #9CA3AF;
    white-space: nowrap;
}}

/* ── Topics Grid ── */
.topics-grid {{
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 20px;
}}
.topic-item {{
    background: #fff;
    padding: 22px;
    border-radius: 14px;
    border: 1px solid #F5E0E8;
    transition: all 0.3s;
    display: flex;
    flex-direction: column;
}}
.topic-item:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(245,169,184,0.1);
}}
.topic-header {{
    display: flex;
    align-items: center;
    margin-bottom: 14px;
}}
.topic-number {{
    background: #5BCEFA;
    color: #fff;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 500;
    margin-right: 14px;
    font-size: 0.9em;
    box-shadow: 0 2px 8px rgba(91,206,250,0.3);
    flex-shrink: 0;
}}
.topic-title {{
    font-weight: 600;
    color: #2D3748;
    font-size: 1.15em;
}}
.topic-contributors {{
    color: #6B7280;
    font-size: 0.88em;
    margin-bottom: 10px;
}}
.topic-detail {{
    color: #374151;
    line-height: 1.65;
    font-size: 0.95em;
}}
.topic-detail .hl-name {{
    color: #F5A9B8;
    font-weight: 600;
}}

/* ── User Titles ── */
.users-grid {{
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 20px;
}}
.user-card {{
    background: #fff;
    padding: 22px;
    border-radius: 14px;
    border: 1px solid #F5E0E8;
    transition: all 0.3s;
}}
.user-card:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(245,169,184,0.12);
}}
.user-card-header {{
    display: flex;
    align-items: center;
    margin-bottom: 12px;
}}
.user-avatar {{
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: #F5A9B8;
    margin-right: 14px;
    flex-shrink: 0;
    object-fit: cover;
}}
.user-name {{
    font-weight: 600;
    color: #2D3748;
    font-size: 1.1em;
    margin-bottom: 6px;
}}
.user-badges {{
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
}}
.badge-title {{
    background: #5BCEFA;
    color: #fff;
    padding: 4px 14px;
    border-radius: 20px;
    font-size: 0.82em;
    font-weight: 500;
}}
.badge-mbti {{
    background: #F5A9B8;
    color: #fff;
    padding: 4px 10px;
    border-radius: 14px;
    font-size: 0.82em;
    font-weight: 500;
}}
.user-habit {{
    font-size: 0.88em;
    color: #5BCEFA;
    margin: 8px 0 4px;
    font-style: italic;
}}
.user-reason {{
    font-size: 0.88em;
    color: #6B7280;
    line-height: 1.5;
}}

/* ── Quotes ── */
.quote-item {{
    background: #FFF5F8;
    padding: 20px 24px;
    margin-bottom: 16px;
    border-radius: 14px;
    border: 1px solid #F5E0E8;
    transition: all 0.3s;
}}
.quote-item:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(245,169,184,0.12);
}}
.quote-content {{
    font-size: 1.08em;
    color: #2D3748;
    font-weight: 500;
    line-height: 1.6;
    margin-bottom: 10px;
    font-style: italic;
}}
.quote-author {{
    font-size: 0.9em;
    color: #F5A9B8;
    font-weight: 600;
    text-align: right;
    margin-bottom: 6px;
}}
.quote-reason {{
    font-size: 0.82em;
    color: #6B7280;
    background: rgba(245,169,184,0.08);
    padding: 8px 12px;
    border-radius: 10px;
    border-left: 3px solid #5BCEFA;
}}

/* ── Relationship Cards ── */
.rel-grid {{
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 18px;
}}
.rel-card {{
    background: #fff;
    border: 1px solid #F5E0E8;
    border-radius: 16px;
    padding: 20px 22px;
    transition: all 0.3s;
}}
.rel-card:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(245,169,184,0.12);
}}
.rel-card-header {{
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    flex-wrap: wrap;
}}
.rel-avatars {{
    display: flex;
    align-items: center;
}}
.rel-avatar {{
    width: 34px;
    height: 34px;
    border-radius: 50%;
    border: 2px solid #fff;
    object-fit: cover;
    margin-left: -8px;
    background: #F5A9B8;
}}
.rel-avatars .rel-avatar:first-child {{
    margin-left: 0;
}}
.rel-badge {{
    padding: 3px 12px;
    border-radius: 20px;
    font-size: 0.78em;
    font-weight: 600;
}}
.rel-badge-duo {{
    background: #F5A9B8;
    color: #fff;
}}
.rel-badge-group {{
    background: #5BCEFA;
    color: #fff;
}}
.rel-label {{
    font-weight: 700;
    font-size: 1.05em;
    color: #2D3748;
}}
.rel-vibe {{
    font-size: 0.9em;
    color: #5BCEFA;
    font-style: italic;
    margin-bottom: 10px;
}}
.rel-evidence {{
    font-size: 0.82em;
    color: #6B7280;
    background: rgba(245,169,184,0.08);
    padding: 8px 12px;
    border-radius: 10px;
    border-left: 3px solid #F5A9B8;
    line-height: 1.5;
    display: flex;
    flex-direction: column;
    gap: 6px;
}}
.rel-evidence-item {{
    padding: 2px 0;
}}
/* ── Footer ── */
.footer {{
    background: #5BCEFA;
    color: #fff;
    text-align: center;
    padding: 30px;
    border-radius: 0 0 24px 24px;
    font-size: 0.9em;
    font-weight: 300;
    letter-spacing: 0.5px;
}}
</style>
</head>
<body>
<div class="container">
    <div class="header">
        <h1>📊 群聊日常分析报告</h1>
        <div class="subtitle">{group_name} · {date_str}</div>
    </div>
    <div class="content">
        <!-- 基础统计 -->
        <div class="section">
            <h2 class="section-title">📈 基础统计</h2>
            <div class="stats-grid">
                <div class="stat-card">
                    <div class="stat-number">{msg_count}</div>
                    <div class="stat-label">消息总数</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number">{participant_count}</div>
                    <div class="stat-label">参与人数</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number">{total_chars}</div>
                    <div class="stat-label">总字符数</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number">{emoji_count}</div>
                    <div class="stat-label">表情数量</div>
                </div>
            </div>
        </div>

        <!-- 亮点一览：活跃时段 + 最热表情 | 发言排行榜 -->
        <div class="section">
            <h2 class="section-title">✨ 亮点一览</h2>
            {highlights_html}
        </div>

        <!-- 24h 活跃度分布 -->
        <div class="section">
            <div class="chart-container">
                <div class="chart-title">⏱️ 24小时活跃度分布</div>
                {hourly_chart}
            </div>
        </div>

        <!-- 热门话题 -->
        {topics_section}

        <!-- 群友称号 -->
        {titles_section}

        <!-- 群圣经 -->
        {quotes_section}

        <!-- 群友关系速写 -->
        {relationships_section}
    </div>
    <div class="footer">
        由 LianBot 生成 · {datetime_str} · Powered by DeepSeek
    </div>
</div>
</body>
</html>"#,
        group_name = html_escape(group_name),
        date_str = date_str,
        datetime_str = datetime_str,
        msg_count = stats.message_count,
        participant_count = stats.participant_count,
        total_chars = stats.total_characters,
        emoji_count = stats.emoji_count,
        highlights_html = highlights_html,
        hourly_chart = hourly_chart,
        topics_section = if llm.topics.is_empty() { String::new() } else {
            format!(r#"<div class="section"><h2 class="section-title">💬 热门话题</h2><div class="topics-grid">{}</div></div>"#, topics_html)
        },
        titles_section = if llm.user_titles.is_empty() { String::new() } else {
            format!(r#"<div class="section"><h2 class="section-title">🏆 群友称号</h2><div class="users-grid">{}</div></div>"#, titles_html)
        },
        quotes_section = if llm.golden_quotes.is_empty() { String::new() } else {
            format!(r#"<div class="section"><h2 class="section-title">💬 群圣经</h2>{}</div>"#, quotes_html)
        },
        relationships_section = if llm.relationships.is_empty() { String::new() } else {
            format!(r#"<div class="section"><h2 class="section-title">🔍 群友关系速写</h2><div class="rel-grid">{}</div></div>"#, relationships_html)
        },
    )
}

// ── 子模块渲染 ────────────────────────────────────────────────────────────────

fn render_relationships(
    relationships: &[super::llm::Relationship],
    name_to_uid: &HashMap<String, i64>,
) -> String {
    relationships
        .iter()
        .map(|r| {
            let badge_class = if r.rel_type == "duo" { "rel-badge-duo" } else { "rel-badge-group" };

            let avatars: String = r.members.iter().map(|name| {
                let src = name_to_uid
                    .get(name)
                    .map(|uid| format!("https://q1.qlogo.cn/g?b=qq&nk={uid}&s=100"))
                    .unwrap_or_default();
                format!(r#"<img class="rel-avatar" src="{src}" alt="">"#)
            }).collect();

            let members_str = r.members.iter().map(|m| html_escape(m)).collect::<Vec<_>>().join(" · ");

            let evidence_html: String = r.evidence.iter().map(|e| {
                format!(r#"<div class="rel-evidence-item">“{}”</div>"#, nl2br(e))
            }).collect();

            format!(
                r#"<div class="rel-card"><div class="rel-card-header"><div class="rel-avatars">{avatars}</div><span class="rel-badge {badge_class}">{members}</span><span class="rel-label">{label}</span></div><div class="rel-vibe">{vibe}</div><div class="rel-evidence">{evidence}</div></div>"#,
                avatars  = avatars,
                badge_class = badge_class,
                members  = members_str,
                label    = html_escape(&r.label),
                vibe     = nl2br(&r.vibe),
                evidence = evidence_html,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染"亮点一览"双栏布局：左栏（活跃时段 + 最热表情），右栏（发言排行榜 Top 3）
fn render_highlights(stats: &Statistics) -> String {
    // 左栏：最活跃时段
    let active_block = format!(
        r#"<div class="active-period"><div class="time">{time}</div><div class="label">最活跃时段</div><div class="active-period-sub">回复 {reply} 条 · @提及 {at} 次</div></div>"#,
        time  = html_escape(&stats.most_active_hour),
        reply = stats.reply_count,
        at    = stats.at_count,
    );

    // 左栏：最热表情（可选）
    let emoji_block = if stats.top_faces.is_empty() {
        String::new()
    } else {
        let items: String = stats.top_faces.iter().map(|(id, cnt)| {
            let emoji = qq_face_to_emoji(id);
            format!(
                r#"<div class="te-item"><span class="te-face">{emoji}</span><span class="te-cnt">×{cnt}</span></div>"#,
                emoji = emoji, cnt = cnt,
            )
        }).collect::<Vec<_>>().join("\n");
        format!(
            r#"<div class="top-emoji-card"><div class="te-label">最热表情</div><div class="te-items">{items}</div></div>"#,
            items = items,
        )
    };

    // 右栏：发言排行榜 Top 3
    let top_count = stats.top_speakers.first().map(|(_, _, c)| *c).unwrap_or(1).max(1);
    let medals = ["🥇", "🥈", "🥉"];
    let rows: String = stats.top_speakers.iter().take(3).enumerate().map(|(i, (uid, name, cnt))| {
        let avatar = format!("https://q1.qlogo.cn/g?b=qq&nk={uid}&s=100");
        let pct = (*cnt as f64 / top_count as f64) * 100.0;
        let medal = medals.get(i).copied().unwrap_or("·");
        format!(
            r#"<div class="speaker-row"><span class="speaker-rank">{medal}</span><img class="speaker-avatar" src="{avatar}" alt=""><div class="speaker-info"><div class="speaker-name">{name}</div><div class="speaker-bar-wrap"><div class="speaker-bar" style="width:{pct:.1}%"></div></div></div><span class="speaker-count">{cnt} 条</span></div>"#,
            medal  = medal,
            avatar = avatar,
            name   = html_escape(name),
            pct    = pct,
            cnt    = cnt,
        )
    }).collect::<Vec<_>>().join("\n");

    let right_col = if rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="leaderboard"><div class="leaderboard-title">🏅 发言排行榜</div>{rows}</div>"#,
            rows = rows,
        )
    };

    format!(
        r#"<div class="highlights-grid"><div class="highlights-left">{active}{emoji}</div>{right}</div>"#,
        active = active_block,
        emoji  = emoji_block,
        right  = right_col,
    )
}

fn render_hourly_chart(hourly: &[u32; 24]) -> String {
    let max = *hourly.iter().max().unwrap_or(&1).max(&1);
    let threshold = 20.0_f64; // 百分比阈值：低于此值把数字放外面

    let mut html = String::new();
    for hour in 0..24 {
        let count = hourly[hour];
        let pct = (count as f64 / max as f64) * 100.0;

        if count > 0 && pct >= threshold {
            html.push_str(&format!(
                r#"<div class="hour-bar-container"><span class="hour-label">{h:02}:00</span><div class="bar-wrapper"><div class="bar" style="width:{pct:.1}%"><span class="bar-value">{count}</span></div></div></div>"#,
                h = hour, pct = pct, count = count
            ));
        } else if count > 0 {
            html.push_str(&format!(
                r#"<div class="hour-bar-container"><span class="hour-label">{h:02}:00</span><div class="bar-wrapper"><div class="bar" style="width:{pct:.1}%"></div><span class="bar-value-outside">{count}</span></div></div>"#,
                h = hour, pct = pct, count = count
            ));
        } else {
            html.push_str(&format!(
                r#"<div class="hour-bar-container"><span class="hour-label">{h:02}:00</span><div class="bar-wrapper"><span class="bar-value-outside">0</span></div></div>"#,
                h = hour
            ));
        }
    }
    html
}

fn render_topics(topics: &[super::llm::Topic]) -> String {
    topics
        .iter()
        .enumerate()
        .map(|(i, t)| {
            // detail 中的 @名字 替换为高亮 span
            let detail_with_br = nl2br(&t.detail);
            let detail_highlighted = highlight_names(&detail_with_br, &t.contributors);
            let contribs = t.contributors.iter().map(|c| html_escape(c)).collect::<Vec<_>>().join("、");
            format!(
                r#"<div class="topic-item"><div class="topic-header"><div class="topic-number">{num}</div><div class="topic-title">{title}</div></div><div class="topic-contributors">参与者: {contribs}</div><div class="topic-detail">{detail}</div></div>"#,
                num = i + 1,
                title = html_escape(&t.topic),
                contribs = contribs,
                detail = detail_highlighted,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_user_titles(titles: &[super::llm::UserTitle], name_to_uid: &HashMap<String, i64>) -> String {
    titles
        .iter()
        .map(|u| {
            let avatar_url = name_to_uid
                .get(&u.name)
                .map(|uid| format!("https://q1.qlogo.cn/g?b=qq&nk={uid}&s=640"))
                .unwrap_or_default();
            format!(
                r#"<div class="user-card"><div class="user-card-header"><img class="user-avatar" src="{avatar}" alt=""><div><div class="user-name">{name}</div><div class="user-badges"><span class="badge-title">{title}</span><span class="badge-mbti">{mbti}</span></div></div></div><div class="user-habit">「{habit}」</div><div class="user-reason">{reason}</div></div>"#,
                avatar = avatar_url,
                name = html_escape(&u.name),
                title = html_escape(&u.title),
                mbti = html_escape(&u.mbti),
                habit = nl2br(&u.habit),
                reason = nl2br(&u.reason),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_quotes(quotes: &[super::llm::Quote]) -> String {
    quotes
        .iter()
        .map(|q| {
            format!(
                r#"<div class="quote-item"><div class="quote-content">“{content}”</div><div class="quote-author">—— {sender}</div><div class="quote-reason">{reason}</div></div>"#,
                content = nl2br(&q.content),
                sender = html_escape(&q.sender),
                reason = nl2br(&q.reason),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 将 detail 中出现的 @名字 替换为高亮 HTML span
fn highlight_names(html_text: &str, contributors: &[String]) -> String {
    let mut result = html_text.to_string();
    for name in contributors {
        let escaped_name = html_escape(name);
        // 替换 @名字 形式
        let pattern = format!("@{}", escaped_name);
        let replacement = format!(r#"<span class="hl-name">{}</span>"#, escaped_name);
        result = result.replace(&pattern, &replacement);
    }
    result
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// HTML 转义后将 `\n` 转为 `<br>`，保留 LLM 输出中的换行。
fn nl2br(s: &str) -> String {
    html_escape(s).replace('\n', "<br>")
}

/// QQ 系统表情 face_id → Unicode emoji 映射。
///
/// 官方参考：https://bot.q.qq.com/wiki/develop/pythonsdk/model/emoji.html
/// 经典 QQ 表情（0-39）+ 官方列表 EmojiType=1 全量收录。
/// 未收录的 ID 回退到带 ID 的占位显示。
fn qq_face_to_emoji(id: &str) -> String {
    let emoji = match id {
        // ── 经典表情（0-39） ──
        "0"   => "😮",      // 惊讶
        "1"   => "😣",      // 撇嘴
        "2"   => "😍",      // 色
        "3"   => "😳",      // 发呆
        "4"   => "😎",      // 得意
        "5"   => "😢",      // 流泪
        "6"   => "☺\u{fe0f}", // 害羞
        "7"   => "🤐",      // 闭嘴
        "8"   => "😪",      // 睡
        "9"   => "😭",      // 大哭
        "10"  => "😅",      // 尴尬
        "11"  => "😡",      // 发怒
        "12"  => "😝",      // 调皮
        "13"  => "😁",      // 呲牙
        "14"  => "😊",      // 微笑
        "15"  => "😞",      // 难过
        "16"  => "😎",      // 酷
        "18"  => "😤",      // 抓狂
        "19"  => "🤮",      // 吐
        "20"  => "🤭",      // 偷笑
        "21"  => "😊",      // 可爱
        "22"  => "🙄",      // 白眼
        "23"  => "😤",      // 傲慢
        "24"  => "😋",      // 饥饿
        "25"  => "😪",      // 困
        "26"  => "😨",      // 惊恐
        "27"  => "😓",      // 流汗
        "28"  => "😄",      // 憨笑
        "29"  => "😌",      // 悠闲
        "30"  => "💪",      // 奋斗
        "31"  => "🤬",      // 咒骂
        "32"  => "🤔",      // 疑问
        "33"  => "🤫",      // 嘘
        "34"  => "😵",      // 晕
        "35"  => "😫",      // 折磨
        "36"  => "😥",      // 衰
        "37"  => "💀",      // 骷髅
        "38"  => "🔨",      // 敲打
        "39"  => "👋",      // 再见
        // ── 经典表情（40-89） ──
        "41"  => "😰",      // 发抖
        "42"  => "❤\u{fe0f}", // 爱情
        "43"  => "🤸",      // 跳跳
        "46"  => "🐷",      // 猪头
        "49"  => "🤗",      // 拥抱
        "53"  => "🎂",      // 蛋糕
        "54"  => "⚡",      // 闪电
        "55"  => "💣",      // 炸弹
        "56"  => "🔪",      // 刀
        "59"  => "💩",      // 便便
        "60"  => "☕",      // 咖啡
        "63"  => "🌹",      // 玫瑰
        "64"  => "🥀",      // 凋谢
        "66"  => "❤\u{fe0f}", // 爱心
        "67"  => "💔",      // 心碎
        "69"  => "🎁",      // 礼物
        "74"  => "☀\u{fe0f}", // 太阳
        "75"  => "🌙",      // 月亮
        "76"  => "👍",      // 赞
        "77"  => "👎",      // 踩
        "78"  => "🤝",      // 握手
        "79"  => "✌\u{fe0f}", // 胜利
        "85"  => "😘",      // 飞吻
        "89"  => "🍉",      // 西瓜
        // ── 扩展表情（96-129） ──
        "96"  => "😰",      // 冷汗
        "97"  => "😓",      // 擦汗
        "98"  => "🤏",      // 抠鼻
        "99"  => "👏",      // 鼓掌
        "100" => "😳",      // 糗大了
        "101" => "😏",      // 坏笑
        "102" => "😤",      // 左哼哼
        "103" => "😤",      // 右哼哼
        "104" => "🥱",      // 哈欠
        "106" => "😢",      // 委屈
        "109" => "😘",      // 左亲亲
        "111" => "🥺",      // 可怜
        "116" => "🥰",      // 示爱
        "118" => "🙏",      // 抱拳
        "120" => "👊",      // 拳头
        "122" => "🤟",      // 爱你
        "123" => "🙅",      // NO
        "124" => "👌",      // OK
        "125" => "🔄",      // 转圈
        "129" => "👋",      // 挥手
        // ── 新版表情（144-183） ──
        "144" => "🥳",      // 喝彩
        "147" => "🍭",      // 棒棒糖
        "171" => "🍵",      // 茶
        "173" => "😭",      // 泪奔
        "174" => "🤷",      // 无奈
        "175" => "🥺",      // 卖萌
        "176" => "😖",      // 小纠结
        "179" => "🐶",      // doge
        "180" => "🤩",      // 惊喜
        "181" => "😏",      // 骚扰
        "182" => "🤣",      // 笑哭
        "183" => "💅",      // 我最美
        // ── 新版表情（201-246） ──
        "201" => "👍",      // 点赞
        "203" => "🤦",      // 托脸
        "212" => "🤔",      // 托腮
        "214" => "😚",      // 啵啵
        "219" => "🥰",      // 蹭一蹭
        "222" => "🤗",      // 抱抱
        "227" => "👏",      // 拍手
        "232" => "🧘",      // 佛系
        "240" => "🤧",      // 喷脸
        "243" => "💆",      // 甩头
        "246" => "🤗",      // 加油抱抱
        // ── 新版表情（262-326） ──
        "262" => "🤯",      // 脑阔疼
        "264" => "🤦",      // 捂脸
        "265" => "🫣",      // 辣眼睛
        "266" => "😯",      // 哦哟
        "267" => "👨\u{200d}🦲", // 头秃
        "268" => "❓",      // 问号脸
        "269" => "👀",      // 暗中观察
        "270" => "😑",      // emm
        "271" => "🍉",      // 吃瓜
        "272" => "😏",      // 呵呵哒
        "273" => "🍋",      // 我酸了
        "277" => "🐶",      // 汪汪
        "278" => "😓",      // 汗
        "281" => "😂",      // 无眼笑
        "282" => "🫡",      // 敬礼
        "284" => "😐",      // 面无表情
        "285" => "🐟",      // 摸鱼
        "287" => "😮",      // 哦
        "289" => "👁\u{fe0f}", // 睁眼
        "290" => "🥳",      // 敲开心
        "293" => "🐟",      // 摸锦鲤
        "294" => "🤩",      // 期待
        "297" => "🙏",      // 拜谢
        "298" => "🧧",      // 元宝
        "299" => "🐮",      // 牛啊
        "305" => "😘",      // 右亲亲
        "306" => "🐮",      // 牛气冲天
        "307" => "🐱",      // 喵喵
        "314" => "🧐",      // 仔细分析
        "315" => "💪",      // 加油
        "318" => "🤩",      // 崇拜
        "319" => "🫶",      // 比心
        "320" => "🎉",      // 庆祝
        "322" => "🙅",      // 拒绝
        "324" => "🍬",      // 吃糖
        "326" => "😡",      // 生气
        _     => return format!("[QQ:{}]", id),
    };
    emoji.to_string()
}
