use std::collections::HashMap;

use crate::commands::{ParamKind, ParamSpec, ValueConstraint};
use crate::runtime::parser::ParamValue;

/// 校验 params 是否符合 specs 声明。返回第一条错误的用户可见文本。
pub fn validate_params(
    params: &HashMap<String, ParamValue>,
    specs: &[ParamSpec],
) -> Result<(), String> {
    // 收集所有已声明的 key
    let declared: std::collections::HashSet<&str> = specs.iter()
        .flat_map(|s| s.keys.iter().copied())
        .collect();

    // 1. 未知参数
    for key in params.keys() {
        if !declared.contains(key.as_str()) {
            return Err(format!("未知参数: {key}"));
        }
    }

    // 2. 必填参数
    for spec in specs {
        if spec.required && !spec.keys.iter().any(|k| params.contains_key(*k)) {
            return Err(format!("缺少必填参数: {}", spec.keys.join(" / ")));
        }
    }

    // 3. 值约束
    for spec in specs {
        if let ParamKind::Value(constraint) = spec.kind {
            for &key in spec.keys {
                if let Some(ParamValue::Value(s)) = params.get(key) {
                    match constraint {
                        ValueConstraint::Any => {}
                        ValueConstraint::Integer { min, max } => {
                            match s.parse::<i64>() {
                                Err(_) => return Err(format!("{key} 需要整数，收到: \"{s}\"")),
                                Ok(n) => {
                                    if let Some(lo) = min {
                                        if n < lo { return Err(format!("{key} 不能小于 {lo}，收到: {n}")); }
                                    }
                                    if let Some(hi) = max {
                                        if n > hi { return Err(format!("{key} 不能大于 {hi}，收到: {n}")); }
                                    }
                                }
                            }
                        }
                        ValueConstraint::OneOf(choices) => {
                            if !choices.contains(&s.as_str()) {
                                return Err(format!("{key} 仅支持: {}，收到: \"{s}\"", choices.join(" / ")));
                            }
                        }
                    }
                    break; // 只校验第一个命中的 key
                }
            }
        }
    }

    Ok(())
}
