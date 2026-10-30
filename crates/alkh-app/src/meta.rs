//! 元数据：枚举选项与内置渲染变量清单，供管理后台渲染表单（`GET /api/v1/meta`）。

use std::collections::BTreeMap;

use alkh_domain::card_catalog::{alert_presets, card_components, CardComponent, CardPreset};
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
    /// 序列化为 `type`（与 `docs/api.md` 一致）；Rust 侧用 `ty` 避开关键字。
    #[serde(rename = "type")]
    pub ty: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub version: &'static str,
    pub repeat_policies: Vec<EnumOption>,
    pub resolved_policies: Vec<EnumOption>,
    pub group_update_policies: Vec<EnumOption>,
    pub match_ops: Vec<EnumOption>,
    pub card_sources: Vec<EnumOption>,
    pub channel_kinds: Vec<EnumOption>,
    pub receive_id_types: Vec<EnumOption>,
    pub variables: Vec<VariableMeta>,
    /// 飞书卡片 JSON 2.0 组件目录：驱动本地卡片构建工具的组件面板/属性面板，
    /// 也是保存时结构校验的依据（见 `alkh_domain::card_catalog`）。
    pub card_components: &'static [CardComponent],
    /// 告警积木（拖进画布即用的高频块，同样来自服务端，保证只用真实变量）。
    pub card_presets: &'static [CardPreset],
    /// 严重级别 → 卡片 header 颜色（对象映射，便于前端直接查表）。
    pub severity_colors: BTreeMap<&'static str, &'static str>,
    pub token_prefix: &'static str,
    pub hook_path_template: &'static str,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

macro_rules! opts {
    ($(($v:literal, $l:literal)),* $(,)?) => { vec![$(EnumOption { value: $v, label: $l }),*] };
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

pub fn card_sources() -> Vec<EnumOption> {
    opts![
        ("builder_json", "卡片搭建工具 JSON（JSON 2.0）"),
        ("builder_template", "引用搭建工具模板（已停用）"),
    ]
}

pub fn channel_kinds() -> Vec<EnumOption> {
    opts![("group", "群聊"), ("p2p_user", "单聊用户")]
}

pub fn receive_id_types() -> Vec<EnumOption> {
    opts![
        ("chat_id", "群 ID（chat_id）"),
        ("open_id", "用户 ID（open_id）"),
        ("user_id", "用户 ID（user_id）"),
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
            name: "alert_group_id",
            group: "group",
            ty: "string",
            description: "告警分组 ID（卡片回调用，如「忽略并 unpin」按钮）",
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
            name: "status_color",
            group: "group",
            ty: "string",
            description: "状态对应卡片颜色（未恢复按严重级别，已恢复 green）",
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
        V {
            name: "has_alerts",
            group: "block",
            ty: "bool",
            description: "本次分组是否有告警（配合「条件」组件使用）",
        },
        // 循环内（`repeat` 组件的子组件里可用，取「当前这一条」告警）
        V {
            name: "item.index",
            group: "item",
            ty: "number",
            description: "当前告警序号（从 1 开始）",
        },
        V {
            name: "item.title",
            group: "item",
            ty: "string",
            description: "当前告警的「名称 · 实例」",
        },
        V {
            name: "item.alertname",
            group: "item",
            ty: "string",
            description: "当前告警的名称",
        },
        V {
            name: "item.instance",
            group: "item",
            ty: "string",
            description: "当前告警的实例",
        },
        V {
            name: "item.job",
            group: "item",
            ty: "string",
            description: "当前告警的 job",
        },
        V {
            name: "item.fingerprint",
            group: "item",
            ty: "string",
            description: "当前告警的指纹",
        },
        V {
            name: "item.status",
            group: "item",
            ty: "string",
            description: "当前告警状态：firing / resolved",
        },
        V {
            name: "item.alert_status",
            group: "item",
            ty: "string",
            description: "同上（与代表告警的 `${alert_status}` 同名，便于迁移）",
        },
        V {
            name: "item.status_text",
            group: "item",
            ty: "string",
            description: "当前告警状态的中文：未解决 / 已恢复",
        },
        V {
            name: "item.status_icon",
            group: "item",
            ty: "string",
            description: "当前告警状态的图标：🔥 / ✅",
        },
        V {
            name: "item.is_firing",
            group: "item",
            ty: "bool",
            description: "当前告警是否未解决",
        },
        V {
            name: "item.status_color",
            group: "item",
            ty: "string",
            description: "当前告警状态对应的卡片颜色",
        },
        V {
            name: "item.severity",
            group: "item",
            ty: "string",
            description: "当前告警的严重级别原值",
        },
        V {
            name: "item.severity_text",
            group: "item",
            ty: "string",
            description: "当前告警严重级别的中文",
        },
        V {
            name: "item.severity_color",
            group: "item",
            ty: "string",
            description: "当前告警严重级别对应的卡片颜色",
        },
        V {
            name: "item.summary",
            group: "item",
            ty: "string",
            description: "当前告警的概要注解",
        },
        V {
            name: "item.description",
            group: "item",
            ty: "string",
            description: "当前告警的详情注解",
        },
        V {
            name: "item.starts_at",
            group: "item",
            ty: "datetime",
            description: "当前告警的开始时间",
        },
        V {
            name: "item.ends_at",
            group: "item",
            ty: "datetime",
            description: "当前告警的结束时间（未结束为空）",
        },
        V {
            name: "item.generator_url",
            group: "item",
            ty: "url",
            description: "当前告警的来源链接",
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
            name: "item.label:<名称>",
            group: "dynamic",
            ty: "string",
            description: "循环内取当前告警的任意标签，如 ${item.label:team}",
        },
        V {
            name: "item.annotation:<名称>",
            group: "dynamic",
            ty: "string",
            description: "循环内取当前告警的任意注解，如 ${item.annotation:runbook}",
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
        repeat_policies: repeat_policies(),
        resolved_policies: resolved_policies(),
        group_update_policies: group_update_policies(),
        match_ops: match_ops(),
        card_sources: card_sources(),
        channel_kinds: channel_kinds(),
        receive_id_types: receive_id_types(),
        variables: render_variables(),
        card_components: card_components(),
        card_presets: alert_presets(),
        severity_colors: [
            ("critical", "red"),
            ("warning", "orange"),
            ("info", "blue"),
            ("default", "grey"),
        ]
        .into_iter()
        .collect(),
        token_prefix: alkh_domain::hook::TOKEN_PREFIX,
        hook_path_template: "/api/v1/hooks/alertmanager/{token}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 从文本里提取 `${...}` **内置变量名**。
    ///
    /// 正则刻意只接受 `[A-Za-z0-9_.]`，因此会自动忽略：
    /// - 带前缀的取值形式 `${label:team}` / `${annotation:x}`（含冒号）
    /// - 占位示例 `${名称}`（非 ASCII）
    fn extract_builtin_variables(text: &str) -> std::collections::BTreeSet<String> {
        let re = regex::Regex::new(r"\$\{([A-Za-z0-9_.]+)\}").expect("正则必须可编译");
        re.captures_iter(text).map(|c| c[1].to_string()).collect()
    }

    /// README §3.3 的变量速查表必须与代码同步。    ///
    /// 这是用户唯一能看到的变量清单，最容易在增删变量时忘记更新——
    /// 而它一旦过期，用户就会照着写不存在的变量（渲染成空串），且毫无提示。
    #[test]
    fn readme_documents_every_variable() {
        const README: &str = include_str!("../../../README.md");
        const START: &str = "<!-- variable-table:start -->";
        const END: &str = "<!-- variable-table:end -->";

        let start = README
            .find(START)
            .expect("README 缺少 variable-table:start 标记")
            + START.len();
        let end = README
            .find(END)
            .expect("README 缺少 variable-table:end 标记");
        assert!(start < end, "README 的变量表标记顺序不对");

        let documented = extract_builtin_variables(&README[start..end]);
        let declared: std::collections::BTreeSet<String> =
            render_variable_names().into_iter().collect();

        let missing: Vec<&String> = declared.difference(&documented).collect();
        assert!(
            missing.is_empty(),
            "README 变量表缺少这些内置变量（新增变量后请同步 README §3.3）：{missing:?}"
        );

        let extra: Vec<&String> = documented.difference(&declared).collect();
        assert!(
            extra.is_empty(),
            "README 变量表出现了代码里不存在的变量（拼写错误？）：{extra:?}"
        );
    }

    /// 告警积木（服务端下发给卡片构建工具）只能引用**真实存在**的变量。
    ///
    /// 积木是最容易被用户直接拖进卡片的路径：如果它引用了拼错的变量，
    /// 渲染出来就是空串，而且没有任何提示。
    #[test]
    fn alert_presets_use_only_builtin_variables() {
        let known: std::collections::BTreeSet<String> =
            render_variable_names().into_iter().collect();
        for preset in alkh_domain::card_catalog::alert_presets() {
            let nodes: serde_json::Value = serde_json::from_str(&preset.nodes_json)
                .unwrap_or_else(|e| panic!("积木「{}」不是合法 JSON: {e}", preset.label));
            for variable in alkh_domain::template::detect_variables_json(&nodes) {
                // `label:` / `annotation:` 是运行时动态 key，不在静态清单里。
                if variable.contains(':') {
                    continue;
                }
                assert!(
                    known.contains(&variable),
                    "积木「{}」引用了未定义变量 ${{{variable}}}",
                    preset.label
                );
            }
        }
    }

    #[test]
    fn builtin_variable_extraction_ignores_prefixed_forms() {
        let found = extract_builtin_variables(
            "${alertname} ${label:team} ${annotation:x} ${名称} ${alerts_list}",
        );
        assert!(found.contains("alertname"));
        assert!(found.contains("alerts_list"));
        assert_eq!(
            found.len(),
            2,
            "带前缀与非 ASCII 的占位符不应被当作内置变量: {found:?}"
        );
    }

    #[test]
    fn all_builtin_variable_names_are_ascii_identifiers() {
        for name in render_variable_names() {
            assert!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
                "内置变量名必须是 ASCII 标识符，便于 README 测试与用户拼写: {name}"
            );
        }
    }

    #[test]
    fn meta_serializes() {
        let v = serde_json::to_value(meta()).unwrap();
        assert!(v["variables"].as_array().unwrap().len() > 20);
        assert!(v["card_components"].as_array().unwrap().len() >= 5);
        assert!(v["card_presets"].as_array().unwrap().len() >= 5);
    }

    /// 回归：前端曾因把 `status_label_types` 当成字符串数组而渲染出重复的
    /// `[object Object]` key。这里把 `/meta` 的**线格式**固定下来。
    #[test]
    fn meta_wire_format_is_enum_option_objects() {
        let v = serde_json::to_value(meta()).unwrap();

        for field in [
            "repeat_policies",
            "resolved_policies",
            "group_update_policies",
            "match_ops",
            "card_sources",
            "channel_kinds",
            "receive_id_types",
        ] {
            let items = v[field]
                .as_array()
                .unwrap_or_else(|| panic!("{field} 必须是数组"));
            assert!(!items.is_empty(), "{field} 不能为空");
            for item in items {
                assert!(
                    item.is_object(),
                    "{field} 的元素必须是 {{value,label}} 对象，实际为 {item}"
                );
                assert!(
                    item["value"].is_string() && item["label"].is_string(),
                    "{field} 的元素必须同时有字符串 value/label，实际为 {item}"
                );
            }
        }

        // 变量必须序列化为 `type`（不是 Rust 字段名 `ty`）
        let var = &v["variables"][0];
        assert!(var["name"].is_string());
        assert!(var["group"].is_string());
        assert!(var["type"].is_string(), "变量应序列化为 `type`: {var}");
        assert!(var.get("ty").is_none(), "不应泄露 Rust 字段名 `ty`: {var}");

        // severity_colors 是对象映射，而不是枚举数组
        let colors = v["severity_colors"]
            .as_object()
            .expect("severity_colors 必须是对象");
        assert_eq!(colors["critical"], "red");
        assert_eq!(colors["warning"], "orange");
        assert_eq!(colors["info"], "blue");
        assert_eq!(colors["default"], "grey");

        assert_eq!(v["token_prefix"], "alkh_");
        assert_eq!(
            v["hook_path_template"],
            "/api/v1/hooks/alertmanager/{token}"
        );
    }

    #[test]
    fn all_enum_values_are_unique() {
        for set in [
            repeat_policies(),
            resolved_policies(),
            group_update_policies(),
            match_ops(),
        ] {
            let mut values: Vec<&str> = set.iter().map(|o| o.value).collect();
            let len = values.len();
            values.sort_unstable();
            values.dedup();
            assert_eq!(values.len(), len, "枚举值重复: {values:?}");
        }
    }
}
