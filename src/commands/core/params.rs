/// 参数值约束，用于 dispatcher 自动校验和 `--help` 生成类型提示。
#[derive(Debug, Clone, Copy)]
pub enum ValueConstraint {
    /// 任意字符串，不校验
    Any,
    /// 整数范围，`min`/`max` 为 `None` 表示无限制
    Integer { min: Option<i64>, max: Option<i64> },
    /// 枚举值，输入必须是其中之一（当前暂无命令使用，保留供未来扩展）
    #[allow(dead_code)]
    OneOf(&'static [&'static str]),
}

/// 参数值类型。
#[derive(Debug, Clone, Copy)]
pub enum ParamKind {
    /// 纯 flag，`--ai`，无值
    Flag,
    /// 携带值，并附带约束条件
    Value(ValueConstraint),
}

/// 单条参数规格说明，供 dispatcher 校验和 `--help` 自动生成使用。
#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    /// 所有键别名，如 `&["-t", "--time"]`
    pub keys: &'static [&'static str],
    pub kind: ParamKind,
    pub required: bool,
    pub help: &'static str,
}
