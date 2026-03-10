/// 将 LLM 回复文本拆分为适合逐条发送的段落列表。
///
/// 策略：
/// 1. 按 `\n\n` 切割为段落
/// 2. 单段超过 `max_chars` → 按句末标点符号再切
/// 3. 短于 `min_chars` 的段与前一段合并
pub fn split_reply(text: &str, max_chars: usize, min_chars: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec![];
    }

    // Step 1: 按空行分段
    let raw_paragraphs: Vec<&str> = text.split("\n\n").collect();

    // Step 2: 长段按句切割
    let mut paragraphs: Vec<String> = Vec::new();
    for para in raw_paragraphs {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if para.len() <= max_chars {
            paragraphs.push(para.to_string());
        } else {
            paragraphs.extend(split_by_sentence(para, max_chars));
        }
    }

    // Step 3: 合并过短的段
    let mut merged: Vec<String> = Vec::new();
    for para in paragraphs {
        if let Some(last) = merged.last_mut() {
            if last.len() < min_chars || para.len() < min_chars {
                last.push('\n');
                last.push_str(&para);
                continue;
            }
        }
        merged.push(para);
    }

    if merged.is_empty() {
        vec![text.to_string()]
    } else {
        merged
    }
}

/// 按句末标点（。！？…）切割长文本，每段不超过 max_chars。
fn split_by_sentence(text: &str, max_chars: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        current.push(ch);
        if is_sentence_end(ch) && current.len() >= max_chars / 3 {
            // 到达一个断句点，且当前已有一定长度
            if current.len() >= max_chars {
                result.push(current.trim().to_string());
                current = String::new();
            }
        }
    }

    if !current.trim().is_empty() {
        // 剩余部分：如果很短，合并到上一段
        if current.trim().len() < max_chars / 4 {
            if let Some(last) = result.last_mut() {
                last.push_str(current.trim());
            } else {
                result.push(current.trim().to_string());
            }
        } else {
            result.push(current.trim().to_string());
        }
    }

    result
}

fn is_sentence_end(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '…' | '!' | '?' | '~' | '～')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_single_segment() {
        let result = split_reply("喵！你好呀", 300, 20);
        assert_eq!(result, vec!["喵！你好呀"]);
    }

    #[test]
    fn paragraphs_split_by_blank_lines() {
        let text = "这是第一段的内容，比较长一些\n\n这是第二段的内容，也有一定长度\n\n这是第三段的内容，同样很充实";
        let result = split_reply(text, 300, 20);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn short_paragraphs_merged() {
        let text = "嗯\n\n好\n\n第三段有一些比较长的内容";
        let result = split_reply(text, 300, 20);
        // "嗯" 和 "好" 都太短，应该合并
        assert!(result.len() < 3);
    }

    #[test]
    fn empty_text() {
        let result = split_reply("", 300, 20);
        assert!(result.is_empty());
    }
}
