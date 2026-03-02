use super::llm::LlmResult;
use super::statistics::Statistics;

// ── HTML 报告渲染 ─────────────────────────────────────────────────────────────
//
// 粉蓝白配色方案:
//   主蓝: #7C9BF5   主粉: #F5A0C0   背景: #F0F4FF
//   卡片: #FFFFFF    文字: #2D3748    次文字: #6B7280
//   渐变: linear-gradient(135deg, #7C9BF5, #F5A0C0)

pub fn render(
    stats: &Statistics,
    llm: &LlmResult,
    group_name: &str,
) -> String {
    let date = chrono::Utc::now() + chrono::Duration::hours(8);
    let date_str = date.format("%Y年%m月%d日").to_string();
    let datetime_str = date.format("%Y-%m-%d %H:%M").to_string();

    let hourly_chart = render_hourly_chart(&stats.hourly_distribution);
    let topics_html = render_topics(&llm.topics);
    let titles_html = render_user_titles(&llm.user_titles);
    let quotes_html = render_quotes(&llm.golden_quotes);

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
    background: #F0F4FF;
    min-height: 100vh;
    padding: 30px;
    line-height: 1.6;
    color: #2D3748;
}}
.container {{
    max-width: 1200px;
    margin: 0 auto;
    background: #FFFFFF;
    border-radius: 24px;
    box-shadow: 0 8px 40px rgba(124,155,245,0.12);
    overflow: hidden;
}}

/* ── Header ── */
.header {{
    background: linear-gradient(135deg, #7C9BF5 0%, #F5A0C0 100%);
    color: #fff;
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
.content {{ padding: 40px 45px; }}
.section {{ margin-bottom: 40px; }}
.section-title {{
    font-size: 1.5em;
    font-weight: 600;
    margin-bottom: 22px;
    color: #4A5568;
    border-bottom: 2px solid #E8EDF8;
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
    background: linear-gradient(135deg, #F8FAFF 0%, #F0F4FF 100%);
    padding: 30px 20px;
    text-align: center;
    border-radius: 16px;
    border: 1px solid #E8EDF8;
    transition: all 0.3s;
}}
.stat-card:hover {{
    transform: translateY(-3px);
    box-shadow: 0 8px 24px rgba(124,155,245,0.15);
}}
.stat-number {{
    font-size: 2.8em;
    font-weight: 300;
    color: #7C9BF5;
    margin-bottom: 6px;
    letter-spacing: -1px;
}}
.stat-label {{
    font-size: 0.9em;
    color: #6B7280;
    text-transform: uppercase;
    letter-spacing: 1px;
}}

/* ── Active Period ── */
.active-period {{
    background: linear-gradient(135deg, #7C9BF5 0%, #F5A0C0 100%);
    color: #fff;
    padding: 35px;
    text-align: center;
    margin: 30px 0;
    border-radius: 18px;
    box-shadow: 0 6px 20px rgba(124,155,245,0.25);
}}
.active-period .time {{
    font-size: 2.8em;
    font-weight: 200;
    margin-bottom: 6px;
}}
.active-period .label {{
    font-size: 0.95em;
    opacity: 0.85;
    letter-spacing: 1px;
    text-transform: uppercase;
}}

/* ── Hourly Chart ── */
.chart-container {{
    background: #FAFBFF;
    padding: 30px;
    border-radius: 18px;
    border: 1px solid #E8EDF8;
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
    background: linear-gradient(90deg, #7C9BF5, #F5A0C0);
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
    border: 1px solid #E8EDF8;
    transition: all 0.3s;
    display: flex;
    flex-direction: column;
}}
.topic-item:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(124,155,245,0.1);
}}
.topic-header {{
    display: flex;
    align-items: center;
    margin-bottom: 14px;
}}
.topic-number {{
    background: linear-gradient(135deg, #7C9BF5, #9FB5F8);
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
    box-shadow: 0 3px 10px rgba(124,155,245,0.3);
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
    border: 1px solid #E8EDF8;
    transition: all 0.3s;
}}
.user-card:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(245,160,192,0.12);
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
    background: linear-gradient(135deg, #F5A0C0, #F8C4D8);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.2em;
    color: #fff;
    margin-right: 14px;
    flex-shrink: 0;
    font-weight: 500;
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
    background: linear-gradient(135deg, #7C9BF5, #9FB5F8);
    color: #fff;
    padding: 4px 14px;
    border-radius: 20px;
    font-size: 0.82em;
    font-weight: 500;
    box-shadow: 0 2px 8px rgba(124,155,245,0.25);
}}
.badge-mbti {{
    background: linear-gradient(135deg, #F5A0C0, #F8C4D8);
    color: #fff;
    padding: 4px 10px;
    border-radius: 14px;
    font-size: 0.82em;
    font-weight: 500;
}}
.user-habit {{
    font-size: 0.88em;
    color: #7C9BF5;
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
    background: linear-gradient(135deg, #FDF2F8 0%, #F0F4FF 100%);
    padding: 20px 24px;
    margin-bottom: 16px;
    border-radius: 14px;
    border: 1px solid #F3E8F0;
    transition: all 0.3s;
}}
.quote-item:hover {{
    transform: translateY(-2px);
    box-shadow: 0 6px 20px rgba(245,160,192,0.12);
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
    color: #F5A0C0;
    font-weight: 600;
    text-align: right;
    margin-bottom: 6px;
}}
.quote-reason {{
    font-size: 0.82em;
    color: #6B7280;
    background: rgba(124,155,245,0.08);
    padding: 8px 12px;
    border-radius: 10px;
    border-left: 3px solid #7C9BF5;
}}

/* ── Footer ── */
.footer {{
    background: linear-gradient(135deg, #7C9BF5 0%, #9FB5F8 100%);
    color: #fff;
    text-align: center;
    padding: 30px;
    font-size: 0.9em;
    font-weight: 300;
    opacity: 0.95;
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
            <div class="active-period">
                <div class="time">{active_hour}</div>
                <div class="label">最活跃时段</div>
            </div>
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
        active_hour = html_escape(&stats.most_active_hour),
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
    )
}

// ── 子模块渲染 ────────────────────────────────────────────────────────────────

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
            let contribs = t.contributors.iter().map(|c| html_escape(c)).collect::<Vec<_>>().join("、");
            format!(
                r#"<div class="topic-item"><div class="topic-header"><div class="topic-number">{num}</div><div class="topic-title">{title}</div></div><div class="topic-contributors">参与者: {contribs}</div><div class="topic-detail">{detail}</div></div>"#,
                num = i + 1,
                title = html_escape(&t.topic),
                contribs = contribs,
                detail = html_escape(&t.detail),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_user_titles(titles: &[super::llm::UserTitle]) -> String {
    titles
        .iter()
        .map(|u| {
            let initial = u.name.chars().next().unwrap_or('?');
            format!(
                r#"<div class="user-card"><div class="user-card-header"><div class="user-avatar">{initial}</div><div><div class="user-name">{name}</div><div class="user-badges"><span class="badge-title">{title}</span><span class="badge-mbti">{mbti}</span></div></div></div><div class="user-habit">「{habit}」</div><div class="user-reason">{reason}</div></div>"#,
                initial = initial,
                name = html_escape(&u.name),
                title = html_escape(&u.title),
                mbti = html_escape(&u.mbti),
                habit = html_escape(&u.habit),
                reason = html_escape(&u.reason),
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
                r#"<div class="quote-item"><div class="quote-content">"{content}"</div><div class="quote-author">—— {sender}</div><div class="quote-reason">{reason}</div></div>"#,
                content = html_escape(&q.content),
                sender = html_escape(&q.sender),
                reason = html_escape(&q.reason),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
