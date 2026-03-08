use crate::commands::{Command, ParamKind, ValueConstraint};

/// 统一检测 `--help` / `-h` 请求。
/// `has_flag` 闭包由调用方提供，适配 Simple（trailing vec）和 Advanced（params map）两种场景。
pub fn try_help(cmd: &dyn Command, has_flag: impl Fn(&str) -> bool) -> Option<String> {
    if has_flag("--help") {
        return Some(format_full_help(cmd));
    }
    if has_flag("-h") {
        return Some(format!("{} — {}", cmd.name(), cmd.help()));
    }
    None
}

/// 完整帮助文本：一行简介 + 自动格式化的参数表（`--help` 触发）。
fn format_full_help(cmd: &dyn Command) -> String {
    let specs = cmd.declared_params();
    let header = format!("{} — {}", cmd.name(), cmd.help());
    if specs.is_empty() {
        return header;
    }
    let mut lines = vec![header, String::new(), "参数：".to_string()];
    for spec in specs {
        let keys = spec.keys.join(", ");
        let type_tag: String = match spec.kind {
            ParamKind::Flag => String::new(),
            ParamKind::Value(ValueConstraint::Any) => " <字符串>".into(),
            ParamKind::Value(ValueConstraint::Integer { min, max }) => match (min, max) {
                (Some(lo), Some(hi)) => format!(" <整数 {lo}-{hi}>"),
                (Some(lo), None)     => format!(" <整数 ≥{lo}>"),
                (None,     Some(hi)) => format!(" <整数 ≤{hi}>"),
                (None,     None)     => " <整数>".into(),
            },
            ParamKind::Value(ValueConstraint::OneOf(choices)) => {
                format!(" <{}>", choices.join("|"))
            }
        };
        let req_tag = if spec.required { "[必填]" } else { "[可选]" };
        let col = format!("{keys}{type_tag}");
        lines.push(format!("  {:<24}  {}  {}", col, req_tag, spec.help));
    }
    lines.join("\n")
}
