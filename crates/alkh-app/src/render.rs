//! 渲染上下文与卡片渲染。

use alkh_domain::alert::{AlertGroup, AlertInstance};
use alkh_domain::channel::Channel;
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::template::{
    render_json, CardMessageSpec, CardSource, RenderContext, RenderedAppFeedCard,
    CARD_JSON_MAX_BYTES,
};
use alkh_domain::Timestamp;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// 渲染后的「卡片消息」内容。
#[derive(Debug, Clone, PartialEq)]
pub enum RenderedCardMessage {
    /// 卡片 JSON 2.0（由 CardKit 创建卡片实体后发送）。
    Json(Value),
    /// 搭建工具模板：直接作为消息 `content`。
    Template {
        template_id: String,
        template_version_name: Option<String>,
        variables: Map<String, Value>,
    },
}

impl RenderedCardMessage {
    /// 作为 `im/v1/messages` 的 `content`（仅 Template 变体使用）。
    pub fn to_content(&self) -> Value {
        match self {
            RenderedCardMessage::Json(card) => card.clone(),
            RenderedCardMessage::Template {
                template_id,
                template_version_name,
                variables,
            } => {
                let mut data = Map::new();
                data.insert("template_id".into(), json!(template_id));
                if let Some(v) = template_version_name {
                    if !v.trim().is_empty() {
                        data.insert("template_version_name".into(), json!(v));
                    }
                }
                data.insert("template_variable".into(), Value::Object(variables.clone()));
                json!({ "type": "template", "data": Value::Object(data) })
            }
        }
    }

    /// 内容摘要：相同即跳过原地更新（R3）。
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(&self.to_content()).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex::encode(&hasher.finalize()[..16])
    }
}

/// 渲染「卡片消息」模板。
pub fn render_card_message(
    spec: &CardMessageSpec,
    ctx: &RenderContext,
) -> AppResult<RenderedCardMessage> {
    match spec.source {
        CardSource::BuilderJson => {
            let card = spec
                .card_json
                .as_ref()
                .ok_or_else(|| AppError::validation("模板缺少 card_json"))?;
            let mut rendered = render_json(card, ctx);
            if !rendered.is_object() {
                return Err(AppError::validation("渲染后的卡片不是 JSON 对象"));
            }
            // 模板保存时只校验了**骨架**，真正的告警数据是这里注入的：含
            // `${alerts_table}` 的卡片在告警很多时很容易超过 30KB，而飞书对超限返回
            // 不可重试的错误 ⇒ 整条告警被标 Failed 丢掉。这里在渲染出口收口体积，
            // 宁可少显示几行文本，也不要让告警消失。
            if shrink_json_to_bytes(&mut rendered, CARD_JSON_MAX_BYTES) {
                tracing::warn!(
                    limit = CARD_JSON_MAX_BYTES,
                    "渲染后的卡片超过飞书体积上限，已截断最长的文本字段"
                );
            }
            Ok(RenderedCardMessage::Json(rendered))
        }
        CardSource::BuilderTemplate => {
            let template_id = spec
                .template_id
                .clone()
                .ok_or_else(|| AppError::validation("模板缺少 template_id"))?;
            Ok(RenderedCardMessage::Template {
                template_id,
                template_version_name: spec.template_version_name.clone(),
                variables: ctx.as_template_variables(&spec.variables),
            })
        }
    }
}

/// 把渲染结果压进飞书卡片体积上限（`CARD_JSON_MAX_BYTES`）。
///
/// 策略：反复找出当前**最长的字符串叶子**并按超出量截断，保证结构始终是合法 JSON。
/// 截断会加一个可见后缀，便于用户知道内容被裁过。
///
/// 返回是否发生了截断。
pub fn shrink_json_to_bytes(value: &mut Value, limit: usize) -> bool {
    const SUFFIX: &str = "…（已截断）";
    let mut truncated = false;
    // 每轮都会砍掉至少 1 字节，设上限防止极端结构下空转。
    for _ in 0..512 {
        let Ok(bytes) = serde_json::to_vec(value) else {
            return truncated;
        };
        if bytes.len() <= limit {
            return truncated;
        }
        let over = bytes.len() - limit;
        let Some((path, len)) = longest_string_leaf(value) else {
            return truncated;
        };
        truncated = true;
        if len <= SUFFIX.len() + over + 2 {
            // 太短了，砍不出多少，直接清空这个叶子。
            set_string_at(value, &path, String::new());
        } else {
            let keep = len - over - SUFFIX.len() - 8;
            let cut = truncate_utf8(
                value_at(value, &path).and_then(Value::as_str).unwrap_or(""),
                keep,
            );
            set_string_at(value, &path, format!("{cut}{SUFFIX}"));
        }
    }
    truncated
}

/// JSON 路径段。
#[derive(Debug, Clone)]
enum Seg {
    Key(String),
    Idx(usize),
}

/// 找出最长的字符串叶子及其路径（按**字节**计，因为上限是字节）。
fn longest_string_leaf(value: &Value) -> Option<(Vec<Seg>, usize)> {
    fn walk(v: &Value, path: &mut Vec<Seg>, best: &mut Option<(Vec<Seg>, usize)>) {
        match v {
            Value::String(s) => {
                if best.as_ref().map(|(_, b)| s.len() > *b).unwrap_or(true) {
                    *best = Some((path.clone(), s.len()));
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(Seg::Idx(i));
                    walk(item, path, best);
                    path.pop();
                }
            }
            Value::Object(map) => {
                for (k, item) in map {
                    path.push(Seg::Key(k.clone()));
                    walk(item, path, best);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut best = None;
    walk(value, &mut Vec::new(), &mut best);
    best
}

fn value_at<'a>(value: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    match path.split_first() {
        None => Some(value),
        Some((Seg::Key(k), rest)) => value_at(value.get(k)?, rest),
        Some((Seg::Idx(i), rest)) => value_at(value.get(*i)?, rest),
    }
}

fn set_string_at(value: &mut Value, path: &[Seg], new: String) {
    match path.split_first() {
        None => {
            if let Value::String(s) = value {
                *s = new;
            }
        }
        Some((Seg::Key(k), rest)) => {
            if let Some(child) = value.get_mut(k) {
                set_string_at(child, rest, new);
            }
        }
        Some((Seg::Idx(i), rest)) => {
            if let Some(child) = value.get_mut(*i) {
                set_string_at(child, rest, new);
            }
        }
    }
}

/// 按字节上限截断，保证落在 UTF-8 字符边界上。
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() > max_bytes {
            break;
        }
        end = i + c.len_utf8();
    }
    &s[..end]
}

/// 渲染「应用消息流卡片」模板。
pub fn render_app_feed_card(
    spec: &alkh_domain::template::AppFeedCardSpec,
    ctx: &RenderContext,
    is_firing: bool,
) -> RenderedAppFeedCard {
    spec.render(ctx, is_firing)
}

/// 严重级别 → 卡片 header 颜色（飞书 `header.template` 合法值）。
pub fn severity_color(severity: Option<&str>) -> &'static str {
    match severity.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("critical") | Some("fatal") | Some("emergency") => "red",
        Some("warning") | Some("warn") | Some("major") => "orange",
        Some("info") | Some("informational") | Some("notice") => "blue",
        _ => "grey",
    }
}

/// 严重级别 → 中文展示。
pub fn severity_text(severity: Option<&str>) -> &'static str {
    match severity.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("critical") | Some("fatal") | Some("emergency") => "严重",
        Some("warning") | Some("warn") | Some("major") => "警告",
        Some("info") | Some("informational") | Some("notice") => "提示",
        _ => "未知",
    }
}

/// 该分组在当前策略下是否携带 firing 告警（决定 `time_sensitive`，R4）。
pub fn is_firing_group(group: &AlertGroup) -> bool {
    group.alerts.iter().any(|a| a.is_firing())
}

const TIME_FMT: &str = "%Y-%m-%d %H:%M:%S UTC";

fn fmt_time(t: Timestamp) -> String {
    t.format(TIME_FMT).to_string()
}

fn fmt_opt_time(t: Option<Timestamp>) -> String {
    t.map(fmt_time).unwrap_or_default()
}

/// 构造渲染上下文（`docs/business-rules.md` §5）。
pub fn build_context(
    group: &AlertGroup,
    channel: &Channel,
    policy: &NotificationPolicy,
    now: Timestamp,
) -> RenderContext {
    let mut ctx = RenderContext::new();
    let is_firing = is_firing_group(group);

    // --- 分组级 ---
    ctx.set("group_key", group.group_key.clone());
    ctx.set("receiver", group.receiver.clone());
    ctx.set(
        "external_url",
        group.external_url.clone().unwrap_or_default(),
    );
    ctx.set("channel_name", channel.name.clone());
    ctx.set("channel_id", channel.receive_id.clone());
    ctx.set("alert_count", group.alerts.len().to_string());
    ctx.set("firing_count", group.firing_count.to_string());
    ctx.set("resolved_count", group.resolved_count.to_string());
    ctx.set("truncated_alerts", group.truncated_alerts.to_string());
    ctx.set("first_seen_at", fmt_time(group.first_seen_at));
    ctx.set("last_seen_at", fmt_time(group.last_seen_at));
    ctx.set("resolved_at", fmt_opt_time(group.resolved_at));
    ctx.set("now", fmt_time(now));
    ctx.set("status", group.status.as_str());
    ctx.set("status_text", if is_firing { "未解决" } else { "已恢复" });
    ctx.set("is_firing", if is_firing { "true" } else { "false" });

    let severity = group.severity.clone();
    ctx.set("severity", severity.clone().unwrap_or_default());
    ctx.set("severity_color", severity_color(severity.as_deref()));
    ctx.set("severity_text", severity_text(severity.as_deref()));

    ctx.set(
        "group_labels",
        serde_json::to_string(&group.group_labels).unwrap_or_else(|_| "{}".into()),
    );
    ctx.set(
        "common_labels",
        serde_json::to_string(&group.common_labels).unwrap_or_else(|_| "{}".into()),
    );
    ctx.set(
        "common_annotations",
        serde_json::to_string(&group.common_annotations).unwrap_or_else(|_| "{}".into()),
    );

    // --- 代表告警（优先取第一条 firing）---
    let representative = group
        .alerts
        .iter()
        .find(|a| a.is_firing())
        .or_else(|| group.alerts.first());

    if let Some(a) = representative {
        ctx.set(
            "alertname",
            a.labels.get("alertname").cloned().unwrap_or_default(),
        );
        ctx.set(
            "instance",
            a.labels.get("instance").cloned().unwrap_or_default(),
        );
        ctx.set("job", a.labels.get("job").cloned().unwrap_or_default());
        ctx.set("fingerprint", a.fingerprint.clone());
        ctx.set(
            "summary",
            a.annotations.get("summary").cloned().unwrap_or_default(),
        );
        ctx.set(
            "description",
            a.annotations
                .get("description")
                .cloned()
                .unwrap_or_default(),
        );
        ctx.set(
            "alert_status",
            if a.is_firing() { "firing" } else { "resolved" },
        );
        ctx.set("starts_at", fmt_opt_time(a.starts_at));
        ctx.set("ends_at", fmt_opt_time(a.ends_at));
        ctx.set("generator_url", a.generator_url.clone().unwrap_or_default());
    } else {
        for k in [
            "alertname",
            "instance",
            "job",
            "fingerprint",
            "summary",
            "description",
            "starts_at",
            "ends_at",
            "generator_url",
            "alert_status",
        ] {
            ctx.set(k, String::new());
        }
        ctx.set("alertname", group.alertname.clone().unwrap_or_default());
    }

    // 若分组级 alertname 缺失，用代表告警的补上。
    if ctx
        .lookup("alertname")
        .map(|s| s.is_empty())
        .unwrap_or(true)
    {
        if let Some(name) = group.alertname.clone() {
            ctx.set("alertname", name);
        }
    }

    // --- 任意 label / annotation 直接可用 + `label:` / `annotation:` 前缀 ---
    let mut set_label = |k: &str, v: &str| {
        if ctx.lookup(k).is_none() {
            ctx.set(k, v.to_string());
        }
        ctx.set(format!("label:{k}"), v.to_string());
    };
    for (k, v) in &group.common_labels {
        set_label(k, v);
    }
    if let Some(a) = representative {
        for (k, v) in &a.labels {
            set_label(k, v);
        }
        for (k, v) in &a.annotations {
            ctx.set(format!("annotation:{k}"), v.clone());
            if ctx.lookup(k).is_none() {
                ctx.set(k, v.to_string());
            }
        }
    }
    for (k, v) in &group.common_annotations {
        // 代表告警自身的注解优先（与 commonLabels/commonAnnotations 的合并语义一致）。
        if ctx.lookup(&format!("annotation:{k}")).is_none() {
            ctx.set(format!("annotation:{k}"), v.clone());
        }
    }

    // --- 预渲染块 ---
    ctx.set("alerts_summary", alerts_summary(group));
    ctx.set(
        "alerts_list",
        alerts_list(group, policy.max_alerts_per_message as usize),
    );
    ctx.set(
        "alerts_table",
        alerts_table(group, policy.max_alerts_per_message as usize),
    );

    ctx
}

fn alerts_summary(group: &AlertGroup) -> String {
    let mut parts = Vec::new();
    if group.firing_count > 0 {
        parts.push(format!("🔥 {} 条未解决", group.firing_count));
    }
    if group.resolved_count > 0 {
        parts.push(format!("✅ {} 条已恢复", group.resolved_count));
    }
    if group.truncated_alerts > 0 {
        parts.push(format!(
            "（另有 {} 条已被 AlertManager 截断）",
            group.truncated_alerts
        ));
    }
    if parts.is_empty() {
        parts.push("无告警".to_string());
    }
    parts.join(" · ")
}

fn status_icon(a: &AlertInstance) -> &'static str {
    if a.is_firing() {
        "🔥"
    } else {
        "✅"
    }
}

fn status_text(a: &AlertInstance) -> &'static str {
    if a.is_firing() {
        "未解决"
    } else {
        "已恢复"
    }
}

fn alert_title(a: &AlertInstance) -> String {
    let name = a
        .labels
        .get("alertname")
        .cloned()
        .unwrap_or_else(|| "告警".into());
    match a.labels.get("instance") {
        Some(inst) if !inst.is_empty() => format!("{name} · `{inst}`"),
        _ => name,
    }
}

fn alerts_list(group: &AlertGroup, max: usize) -> String {
    if group.alerts.is_empty() {
        return "无告警".into();
    }
    let shown = group.alerts.iter().take(max);
    let mut lines: Vec<String> = Vec::new();
    for a in shown {
        lines.push(format!(
            "{} **{}**（{}）",
            status_icon(a),
            alert_title(a),
            status_text(a)
        ));
        if let Some(s) = a
            .annotations
            .get("summary")
            .filter(|s| !s.trim().is_empty())
        {
            lines.push(format!("　‣ 概要：{s}"));
        }
        if let Some(s) = a
            .annotations
            .get("description")
            .filter(|s| !s.trim().is_empty())
        {
            lines.push(format!("　‣ 详情：{}", truncate(s, 200)));
        }
        if a.is_firing() {
            lines.push(format!("　‣ 开始：{}", fmt_opt_time(a.starts_at)));
        } else {
            lines.push(format!("　‣ 恢复：{}", fmt_opt_time(a.ends_at)));
        }
    }
    let hidden = group.alerts.len().saturating_sub(max);
    if hidden > 0 {
        lines.push(format!("> 另有 **{hidden}** 条告警未在此展示"));
    }
    if group.truncated_alerts > 0 {
        lines.push(format!(
            "> AlertManager 另外截断了 **{}** 条告警",
            group.truncated_alerts
        ));
    }
    lines.join("\n")
}

fn alerts_table(group: &AlertGroup, max: usize) -> String {
    if group.alerts.is_empty() {
        return "无告警".into();
    }
    let mut out = String::from("| 状态 | 告警 | 实例 | 概要 |\n| --- | --- | --- | --- |\n");
    for a in group.alerts.iter().take(max) {
        out.push_str(&format!(
            "| {} {} | {} | {} | {} |\n",
            status_icon(a),
            status_text(a),
            escape_cell(a.labels.get("alertname").map(String::as_str).unwrap_or("-")),
            escape_cell(a.labels.get("instance").map(String::as_str).unwrap_or("-")),
            escape_cell(
                a.annotations
                    .get("summary")
                    .map(String::as_str)
                    .unwrap_or("-")
            ),
        ));
    }
    let hidden = group.alerts.len().saturating_sub(max);
    if hidden > 0 {
        out.push_str(&format!("\n> 另有 **{hidden}** 条告警未展示\n"));
    }
    out
}

fn escape_cell(s: &str) -> String {
    truncate(s, 60).replace('|', "\\|").replace('\n', " ")
}

fn truncate(s: &str, max: usize) -> String {
    let cleaned = s.trim();
    if cleaned.chars().count() <= max {
        return cleaned.to_string();
    }
    let mut out: String = cleaned.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alkh_domain::alert::{AlertGroup, AlertManagerPayload, NormalizedEvent};
    use alkh_domain::channel::{Channel, ChannelSource};
    use alkh_domain::ids::{ChannelId, HookId};
    use alkh_domain::policy::NotificationPolicy;
    use alkh_domain::template::{detect_variables_json, CardMessageSpec};

    fn sample_group() -> AlertGroup {
        let payload: AlertManagerPayload = serde_json::from_value(serde_json::json!({
            "version": "4",
            "groupKey": "{}:{alertname=\"HighCPU\"}",
            "status": "firing",
            "receiver": "lark-ops",
            "commonLabels": {"alertname": "HighCPU", "severity": "critical", "job": "node"},
            "commonAnnotations": {"summary": "CPU 过高"},
            "externalURL": "http://am:9093",
            "alerts": [
                {"status":"firing","labels":{"alertname":"HighCPU","instance":"10.0.0.1:9100"},
                 "annotations":{"summary":"CPU 92%","description":"持续 5 分钟"},
                 "startsAt":"2026-10-13T00:00:00Z","endsAt":"0001-01-01T00:00:00Z","fingerprint":"fp1"},
                {"status":"resolved","labels":{"alertname":"HighCPU","instance":"10.0.0.2:9100"},
                 "annotations":{"summary":"已恢复"},"fingerprint":"fp2"}
            ]
        }))
        .unwrap();
        let event = NormalizedEvent::from_payload(&payload).unwrap();
        AlertGroup::new(HookId::new(), ChannelId::new(), &event, chrono::Utc::now())
    }

    fn sample_channel() -> Channel {
        Channel::from_chat("oc_1", "运维告警群", None, false)
    }

    /// 未超限的卡片必须原样返回（不能误伤正常内容）。
    #[test]
    fn shrink_leaves_small_cards_untouched() {
        let mut card = json!({"a": "短文本", "b": {"c": [1, 2, 3]}});
        let before = card.clone();
        assert!(!shrink_json_to_bytes(&mut card, 30 * 1024));
        assert_eq!(card, before);
    }

    /// 超限时必须压到上限内、仍是合法 JSON 对象，并留下可见的截断标记。
    #[test]
    fn shrink_truncates_longest_string_within_limit() {
        let mut card = json!({
            "header": {"title": "小标题"},
            "body": {"elements": [
                {"tag": "markdown", "content": "x".repeat(50_000)},
                {"tag": "markdown", "content": "y".repeat(100)}
            ]}
        });
        assert!(shrink_json_to_bytes(&mut card, 4096));
        let bytes = serde_json::to_vec(&card).unwrap().len();
        assert!(bytes <= 4096, "压缩后仍超限: {bytes}");

        let long = card["body"]["elements"][0]["content"].as_str().unwrap();
        assert!(long.contains("已截断"), "应留可见标记");
        let short = card["body"]["elements"][1]["content"].as_str().unwrap();
        assert_eq!(short, "y".repeat(100), "短字段不应被动");
        assert_eq!(card["header"]["title"], "小标题");
    }

    /// 截断必须落在 UTF-8 字符边界上（中文内容不能被切成半个字符）。
    #[test]
    fn shrink_never_splits_multibyte_characters() {
        let long = "中文告警Ⓜ️".repeat(4_000);
        let mut card = json!({"content": long.clone()});
        assert!(shrink_json_to_bytes(&mut card, 2048));
        let out = card["content"].as_str().unwrap();
        assert!(serde_json::to_vec(&json!({"content": out})).unwrap().len() <= 2048);
        // 能取到 str 本身就意味着没有切断 UTF-8；再确认前缀未被篡改
        assert!(
            long.starts_with(out.trim_end_matches("…（已截断）")),
            "前缀应保持原样"
        );
    }

    /// 没有可截断的字符串时不应死循环或 panic。
    #[test]
    fn shrink_gives_up_when_nothing_can_be_truncated() {
        let mut card = json!({"a": [1, 2, 3], "b": {"c": true}, "d": null});
        assert!(!shrink_json_to_bytes(&mut card, 8));
    }

    #[test]
    fn context_exposes_documented_variables() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(&group, &channel, &policy, chrono::Utc::now());

        assert_eq!(ctx.lookup("status_text"), Some("未解决"));
        assert_eq!(ctx.lookup("status"), Some("firing"));
        assert_eq!(ctx.lookup("severity_color"), Some("red"));
        assert_eq!(ctx.lookup("alertname"), Some("HighCPU"));
        assert_eq!(ctx.lookup("channel_name"), Some("运维告警群"));
        assert_eq!(ctx.lookup("instance"), Some("10.0.0.1:9100"));
        assert_eq!(ctx.lookup("summary"), Some("CPU 92%"));
        assert_eq!(ctx.lookup("job"), Some("node"));
        assert_eq!(ctx.lookup("label:severity"), Some("critical"));
        assert_eq!(ctx.lookup("annotation:summary"), Some("CPU 92%"));
        assert_eq!(ctx.lookup("alert_count"), Some("2"));
        assert!(ctx.lookup("alerts_list").unwrap().contains("HighCPU"));
    }

    #[test]
    fn representative_alert_prefers_firing() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(&group, &channel, &policy, chrono::Utc::now());
        assert_eq!(ctx.lookup("instance"), Some("10.0.0.1:9100"));
    }

    #[test]
    fn default_card_renders_without_leftover_placeholders() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(&group, &channel, &policy, chrono::Utc::now());

        let spec = CardMessageSpec::builder_json(crate::defaults::default_card_json());
        let rendered = render_card_message(&spec, &ctx).unwrap();
        let text = serde_json::to_string(&rendered.to_content()).unwrap();

        // 默认卡片引用的所有变量都在上下文中存在
        for v in detect_variables_json(&crate::defaults::default_card_json()) {
            assert!(ctx.lookup(&v).is_some(), "缺少变量 {v}");
        }
        assert!(text.contains("HighCPU"));
        assert!(!text.contains("${"), "渲染后仍残留占位符: {text}");
    }

    #[test]
    fn severity_mapping_matches_header_enum() {
        assert_eq!(severity_color(Some("critical")), "red");
        assert_eq!(severity_color(Some("WARNING")), "orange");
        assert_eq!(severity_color(Some("info")), "blue");
        assert_eq!(severity_color(None), "grey");
        assert_eq!(severity_color(Some("nonsense")), "grey");
    }

    #[test]
    fn digest_is_stable_and_content_sensitive() {
        let a = RenderedCardMessage::Json(json!({"a": 1}));
        let b = RenderedCardMessage::Json(json!({"a": 1}));
        let c = RenderedCardMessage::Json(json!({"a": 2}));
        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), c.digest());
    }

    #[test]
    fn template_message_builds_documented_content() {
        let mut spec = CardMessageSpec::builder_template("AAq123", Some("1.0.0".into()));
        spec.variables = vec![alkh_domain::template::TemplateVariable::custom(
            "alertname",
            alkh_domain::template::VariableType::String,
        )];
        let ctx = RenderContext::new().with("alertname", "HighCPU");
        let rendered = render_card_message(&spec, &ctx).unwrap();
        assert_eq!(
            rendered.to_content(),
            json!({"type":"template","data":{"template_id":"AAq123","template_version_name":"1.0.0","template_variable":{"alertname":"HighCPU"}}})
        );
    }

    #[test]
    fn max_alerts_truncates_list() {
        let group = sample_group();
        let channel = sample_channel();
        let mut policy = NotificationPolicy::default_for(channel.id.clone());
        policy.max_alerts_per_message = 1;
        let ctx = build_context(&group, &channel, &policy, chrono::Utc::now());
        let list = ctx.lookup("alerts_list").unwrap();
        assert!(list.contains("另有 **1** 条告警未在此展示"), "{list}");
    }

    #[test]
    fn resolved_group_reports_resolved_status() {
        let payload: AlertManagerPayload = serde_json::from_value(serde_json::json!({
            "version":"4","groupKey":"g","status":"resolved","receiver":"r",
            "commonLabels":{"alertname":"X","severity":"warning"},
            "alerts":[{"status":"resolved","labels":{"alertname":"X"},"annotations":{},"fingerprint":"f"}]
        }))
        .unwrap();
        let event = NormalizedEvent::from_payload(&payload).unwrap();
        let group = AlertGroup::new(HookId::new(), ChannelId::new(), &event, chrono::Utc::now());
        let channel = Channel::new(
            "u",
            "ou_1",
            alkh_domain::channel::ReceiveIdType::OpenId,
            alkh_domain::channel::ChannelKind::P2pUser,
            ChannelSource::Manual,
        );
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(&group, &channel, &policy, chrono::Utc::now());
        assert_eq!(ctx.lookup("status_text"), Some("已恢复"));
        assert_eq!(ctx.lookup("is_firing"), Some("false"));
        assert_eq!(ctx.lookup("severity_color"), Some("orange"));
        assert_eq!(ctx.lookup("status"), Some("resolved"));
    }
}
