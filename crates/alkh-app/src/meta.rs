//! 元数据：枚举选项与内置渲染变量清单，供管理后台渲染表单（`GET /api/v1/meta`）。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EnumOption {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariableMeta {
    pub name: &'static str,
    /// `alert` | `group` | `block` | `dynamic`
    pub group: &'static str,
    pub ty: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub version: &'static str,
    pub message_types: Vec<EnumOption>,
    pub repeat_policies: Vec<EnumOption>,
    pub resolved_policies: Vec<EnumOption>,
    pub group_update_policies: Vec<EnumOption>,
    pub match_ops: Vec<EnumOption>,
    pub status_label_types: Vec<EnumOption>,
    pub card_sources: Vec<EnumOption>,
    pub channel_kinds: Vec<EnumOption>,
    pub receive_id_types: Vec<EnumOption>,
    pub variables: Vec<VariableMeta>,
    pub severity_colors: Vec<EnumOption>,
    /// 应用消息流卡片 `pin_unresolved` 的有效性提示（R4）。
    pub pin_supported_message_types: Vec<&'static str>,
    pub token_prefix: &'static str,
    pub hook_path_template: &'static str,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! opts {
    ($(($v:literal, $l:literal)),* $(,)?) => { vec![$(EnumOption { value: $v, label: $l }),*] };
}

pub fn message_types() -> Vec<EnumOption> {
    opts![
        ("card_message", "卡片消息"),
        ("app_feed_card", "应用消息流消息")
    ]
}

pub fn repeat_policies() -> Vec<EnumOption> {
    opts![
        ("suppress", "不发送"),
        ("send", "发送"),
        ("send_and_revoke_previous", "发送并移除旧告警"),
    ]
}

pub fn resolved_policies() -> Vec<EnumOption> {
    opts![
        ("send", "发送"),
        ("send_and_delete_previous", "发送并删除旧告警"),
        ("update_previous", "更新旧告警"),
    ]
}

pub fn group_update_policies() -> Vec<EnumOption> {
    opts![
        ("update_previous", "更新旧告警"),
        ("send", "发送新消息"),
        ("suppress", "不发送"),
    ]
}

pub fn match_ops() -> Vec<EnumOption> {
    opts![
        ("=", "等于"),
        ("!=", "不等于"),
        ("=~", "正则匹配"),
        ("!~", "正则不匹配"),
    ]
}

pub fn status_label_types() -> Vec<EnumOption> {
    opts![
        ("primary", "主类型"),
        ("secondary", "次要类型"),
        ("success", "成功类型"),
        ("danger", "危险类型"),
    ]
}

pub fn card_sources() -> Vec<EnumOption> {
    opts![
        ("builder_json", "卡片搭建工具 JSON（JSON 2.0）"),
        ("builder_template", "卡片搭建工具模板（template_id）"),
    ]
}

pub fn channel_kinds() -> Vec<EnumOption> {
    opts![("group", "群聊"), ("p2p_user", "单聊用户")]
}

pub fn receive_id_types() -> Vec<EnumOption> {
    opts![
        ("chat_id", "群 chat_id"),
        ("open_id", "用户 open_id"),
        ("user_id", "用户 user_id"),
        ("email", "用户邮箱"),
    ]
}

/// 内置渲染变量清单（`docs/business-rules.md` §5）。
pub fn render_variables() -> Vec<VariableMeta> {
    use VariableMeta as V;
    vec![
        // 分组级
        V {
            name: "group_key",
            group: "group",
            ty: "string",
            description: "AlertManager 分组键",
        },
        V {
            name: "receiver",
            group: "group",
            ty: "string",
            description: "AlertManager receiver 名",
        },
        V {
            name: "external_url",
            group: "group",
            ty: "url",
            description: "AlertManager 地址",
        },
        V {
            name: "channel_name",
            group: "group",
            ty: "string",
            description: "消息渠道名称",
        },
        V {
            name: "channel_id",
            group: "group",
            ty: "string",
            description: "渠道接收者 ID",
        },
        V {
            name: "alert_count",
            group: "group",
            ty: "number",
            description: "本次告警条数",
        },
        V {
            name: "firing_count",
            group: "group",
            ty: "number",
            description: "未解决告警条数",
        },
        V {
            name: "resolved_count",
            group: "group",
            ty: "number",
            description: "已恢复告警条数",
        },
        V {
            name: "truncated_alerts",
            group: "group",
            ty: "number",
            description: "被 AlertManager 截断的条数",
        },
        V {
            name: "first_seen_at",
            group: "group",
            ty: "datetime",
            description: "分组首次出现时间",
        },
        V {
            name: "last_seen_at",
            group: "group",
            ty: "datetime",
            description: "分组最后更新时间",
        },
        V {
            name: "resolved_at",
            group: "group",
            ty: "datetime",
            description: "恢复时间（未恢复为空）",
        },
        V {
            name: "now",
            group: "group",
            ty: "datetime",
            description: "当前时间",
        },
        V {
            name: "status",
            group: "group",
            ty: "string",
            description: "firing / resolved",
        },
        V {
            name: "status_text",
            group: "group",
            ty: "string",
            description: "未解决 / 已恢复",
        },
        V {
            name: "is_firing",
            group: "group",
            ty: "bool",
            description: "是否仍存在未解决告警",
        },
        V {
            name: "severity",
            group: "group",
            ty: "string",
            description: "严重级别原始值",
        },
        V {
            name: "severity_text",
            group: "group",
            ty: "string",
            description: "严重级别中文",
        },
        V {
            name: "severity_color",
            group: "group",
            ty: "string",
            description: "严重级别对应卡片颜色（red/orange/blue/grey）",
        },
        V {
            name: "group_labels",
            group: "group",
            ty: "json",
            description: "groupLabels JSON",
        },
        V {
            name: "common_labels",
            group: "group",
            ty: "json",
            description: "commonLabels JSON",
        },
        V {
            name: "common_annotations",
            group: "group",
            ty: "json",
            description: "commonAnnotations JSON",
        },
        // 代表告警
        V {
            name: "alertname",
            group: "alert",
            ty: "string",
            description: "告警名称",
        },
        V {
            name: "instance",
            group: "alert",
            ty: "string",
            description: "实例",
        },
        V {
            name: "job",
            group: "alert",
            ty: "string",
            description: "job",
        },
        V {
            name: "fingerprint",
            group: "alert",
            ty: "string",
            description: "告警指纹",
        },
        V {
            name: "summary",
            group: "alert",
            ty: "string",
            description: "概要 annotation",
        },
        V {
            name: "description",
            group: "alert",
            ty: "string",
            description: "详情 annotation",
        },
        V {
            name: "alert_status",
            group: "alert",
            ty: "string",
            description: "代表告警状态",
        },
        V {
            name: "starts_at",
            group: "alert",
            ty: "datetime",
            description: "代表告警开始时间",
        },
        V {
            name: "ends_at",
            group: "alert",
            ty: "datetime",
            description: "代表告警结束时间",
        },
        V {
            name: "generator_url",
            group: "alert",
            ty: "url",
            description: "告警来源链接",
        },
        // 预渲染块
        V {
            name: "alerts_summary",
            group: "block",
            ty: "markdown",
            description: "一行汇总，如 🔥 3 条未解决 · ✅ 1 条已恢复",
        },
        V {
            name: "alerts_list",
            group: "block",
            ty: "markdown",
            description: "告警列表（推荐）",
        },
        V {
            name: "alerts_table",
            group: "block",
            ty: "markdown",
            description: "告警 Markdown 表格",
        },
        // 动态
        V {
            name: "label:<名称>",
            group: "dynamic",
            ty: "string",
            description: "任意标签，如 ${label:team}",
        },
        V {
            name: "annotation:<名称>",
            group: "dynamic",
            ty: "string",
            description: "任意注解，如 ${annotation:runbook}",
        },
        V {
            name: "<名称>",
            group: "dynamic",
            ty: "string",
            description: "标签/注解可直接用原名引用，如 ${team}",
        },
    ]
}

/// 全部可用变量名（用于校验卡片模板）。
pub fn render_variable_names() -> Vec<String> {
    render_variables()
        .iter()
        .filter(|v| v.group != "dynamic")
        .map(|v| v.name.to_string())
        .collect()
}

pub fn meta() -> Meta {
    Meta {
        version: VERSION,
        message_types: message_types(),
        repeat_policies: repeat_policies(),
        resolved_policies: resolved_policies(),
        group_update_policies: group_update_policies(),
        match_ops: match_ops(),
        status_label_types: status_label_types(),
        card_sources: card_sources(),
        channel_kinds: channel_kinds(),
        receive_id_types: receive_id_types(),
        variables: render_variables(),
        severity_colors: opts![
            ("critical", "red"),
            ("warning", "orange"),
            ("info", "blue"),
            ("default", "grey"),
        ],
        // R4：只有应用消息流卡片支持 time_sensitive 即时提醒。
        pin_supported_message_types: vec!["app_feed_card"],
        token_prefix: alkh_domain::hook::TOKEN_PREFIX,
        hook_path_template: "/api/v1/hooks/alertmanager/{token}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_serializes() {
        let m = meta();
        let v = serde_json::to_value(&m).unwrap();
        assert!(v["message_types"].as_array().unwrap().len() == 2);
        assert!(v["variables"].as_array().unwrap().len() > 20);
        assert_eq!(v["pin_supported_message_types"][0], "app_feed_card");
    }

    #[test]
    fn all_enum_values_are_unique() {
        for set in [
            message_types(),
            repeat_policies(),
            resolved_policies(),
            group_update_policies(),
            match_ops(),
            status_label_types(),
        ] {
            let mut values: Vec<&str> = set.iter().map(|o| o.value).collect();
            let len = values.len();
            values.sort_unstable();
            values.dedup();
            assert_eq!(values.len(), len, "枚举值重复: {values:?}");
        }
    }
}
