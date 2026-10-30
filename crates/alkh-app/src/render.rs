//! 渲染上下文与卡片渲染。

use alkh_domain::alert::{AlertGroup, AlertInstance};
use alkh_domain::channel::Channel;
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::settings::DisplayTimeZone;
use alkh_domain::template::{
    render_str, CardMessageSpec, CardSource, RenderContext, CARD_JSON_MAX_BYTES,
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
            let mut rendered = expand_card_json(card, ctx);
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
/// 策略：每轮把**超出的字节数**从最长的字符串叶子开始逐段砍掉（一轮可以砍多个叶子），
/// 保证结构始终是合法 JSON。截断会加一个可见后缀，便于用户知道内容被裁过。
///
/// 返回是否发生了截断。
pub fn shrink_json_to_bytes(value: &mut Value, limit: usize) -> bool {
    const SUFFIX: &str = "…（已截断）";
    let mut truncated = false;
    // 每轮都**一次性砍够**（而不是每轮只砍一个叶子）：否则「大量短字符串」的长尾
    // 会在轮次上限内收敛不了，卡片仍然超限 → 飞书直接拒收、整条告警丢失。
    // 每轮都让序列化体积严格变小，几轮内必然收敛（8 轮纯粹是保险）。
    for _ in 0..8 {
        let Ok(bytes) = serde_json::to_vec(value) else {
            return truncated;
        };
        if bytes.len() <= limit {
            return truncated;
        }
        // 需要腾出的字节数（+8 字节余量）。截断按**原始字符串字节**估算，而 JSON
        // 转义只会让实际释放更多，所以这个估算天然偏保守，不会「砍完还差一点」。
        let need = (bytes.len() - limit).saturating_add(8);

        let mut leaves = string_leaves(value);
        leaves.sort_by_key(|leaf| std::cmp::Reverse(leaf.1)); // 先砍最长的，尽量保留短的结构字段
        let mut changed = false;
        let mut remaining = need;
        for (path, len) in leaves {
            if remaining == 0 {
                break;
            }
            if len == 0 {
                continue;
            }
            changed = true;
            truncated = true;
            if len <= remaining.saturating_add(SUFFIX.len()) {
                // 砍光整段也腾不出多少，直接清空这个叶子。
                set_string_at(value, &path, String::new());
                remaining = remaining.saturating_sub(len);
            } else {
                // 保留可见后缀（饱和减法：`remaining + 后缀` 可能大于字符串长度）。
                let current = value_at(value, &path).and_then(Value::as_str).unwrap_or("");
                let keep = current.len().saturating_sub(remaining + SUFFIX.len());
                let cut = truncate_utf8(current, keep);
                set_string_at(value, &path, format!("{cut}{SUFFIX}"));
                remaining = 0;
            }
        }
        if !changed {
            return truncated; // 已经没有可压缩的字符串了
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

/// 收集所有字符串叶子及其路径（按**字节**计，因为上限是字节）。
fn string_leaves(value: &Value) -> Vec<(Vec<Seg>, usize)> {
    fn walk(v: &Value, path: &mut Vec<Seg>, out: &mut Vec<(Vec<Seg>, usize)>) {
        match v {
            Value::String(s) => out.push((path.clone(), s.len())),
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(Seg::Idx(i));
                    walk(item, path, out);
                    path.pop();
                }
            }
            Value::Object(map) => {
                for (k, item) in map {
                    path.push(Seg::Key(k.clone()));
                    walk(item, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut Vec::new(), &mut out);
    out
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

/// 严重级别 → 卡片 header 颜色（飞书 `header.template` 合法值）。
pub fn severity_color(severity: Option<&str>) -> &'static str {
    match severity.map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("critical") | Some("fatal") | Some("emergency") => "red",
        Some("warning") | Some("warn") | Some("major") => "orange",
        Some("info") | Some("informational") | Some("notice") => "blue",
        _ => "grey",
    }
}

/// 分组状态 → 卡片 header 颜色。
///
/// 还有未解决告警时按严重级别配色（`severity_color`）；**全部恢复后统一为 `green`**，
/// 否则「已恢复」的卡片会保留原来的告警色（如 warning 的 orange），标题颜色不随状态变化。
pub fn status_color(is_firing: bool, severity: Option<&str>) -> &'static str {
    if is_firing {
        severity_color(severity)
    } else {
        "green"
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

/// 构造渲染上下文（`docs/business-rules.md` §5）。
///
/// `tz` 为卡片内时间的展示时区（来自应用设置，默认 UTC）；
/// AlertManager 数据层的时间戳始终是 UTC，这里只做展示换算。
pub fn build_context(
    group: &AlertGroup,
    channel: &Channel,
    policy: &NotificationPolicy,
    now: Timestamp,
    tz: DisplayTimeZone,
) -> RenderContext {
    let mut ctx = RenderContext::new();
    let is_firing = is_firing_group(group);

    // --- 分组级 ---
    ctx.set("group_key", group.group_key.clone());
    ctx.set("alert_group_id", group.id.to_string());
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
    ctx.set("first_seen_at", tz.format(group.first_seen_at));
    ctx.set("last_seen_at", tz.format(group.last_seen_at));
    ctx.set("resolved_at", tz.format_opt(group.resolved_at));
    ctx.set("now", tz.format(now));
    ctx.set("status", group.status.as_str());
    ctx.set("status_text", if is_firing { "未解决" } else { "已恢复" });
    ctx.set("is_firing", if is_firing { "true" } else { "false" });

    let severity = group.severity.clone();
    ctx.set("severity", severity.clone().unwrap_or_default());
    ctx.set("severity_color", severity_color(severity.as_deref()));
    ctx.set("status_color", status_color(is_firing, severity.as_deref()));
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
        ctx.set("starts_at", tz.format_opt(a.starts_at));
        ctx.set("ends_at", tz.format_opt(a.ends_at));
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
        alerts_list(group, policy.max_alerts_per_message as usize, tz),
    );
    ctx.set(
        "alerts_table",
        alerts_table(group, policy.max_alerts_per_message as usize),
    );
    ctx.set(
        "has_alerts",
        if group.alerts.is_empty() {
            "false"
        } else {
            "true"
        },
    );

    // --- 结构化数据：供本地构建器指令（循环/条件）在渲染期使用 ---
    // 每条告警都已预格式化成展示用的字符串，循环体里直接写 `${item.xxx}` 即可；
    // 大数组放在 `data` 通道里，不会出现在预览的变量面板中。
    ctx.set_data(
        "alerts",
        Value::Array(group.alerts.iter().map(|a| alert_view(a, tz)).collect()),
    );
    ctx.set_data("alerts_limit", json!(policy.max_alerts_per_message));
    ctx.set_data("truncated_alerts", json!(group.truncated_alerts));

    ctx
}

/// 把一条告警转成循环体可用的展示视图（时间/颜色等都已算好）。
fn alert_view(alert: &AlertInstance, tz: DisplayTimeZone) -> Value {
    let mut view = serde_json::to_value(alert).unwrap_or_else(|_| json!({}));
    let severity = alert.labels.get("severity").map(String::as_str);
    if let Some(obj) = view.as_object_mut() {
        obj.insert("status_text".into(), json!(status_text(alert)));
        obj.insert("status_icon".into(), json!(status_icon(alert)));
        obj.insert("is_firing".into(), json!(alert.is_firing()));
        obj.insert("title".into(), json!(alert_title(alert)));
        obj.insert("severity".into(), json!(severity.unwrap_or_default()));
        obj.insert("severity_text".into(), json!(severity_text(severity)));
        obj.insert("severity_color".into(), json!(severity_color(severity)));
        obj.insert(
            "status_color".into(),
            json!(status_color(alert.is_firing(), severity)),
        );
        obj.insert("summary".into(), json!(annotation(alert, "summary")));
        obj.insert(
            "description".into(),
            json!(annotation(alert, "description")),
        );
        // 与 `${starts_at}` / `${ends_at}` 同格式（RFC3339 对用户不友好）。
        obj.insert("starts_at".into(), json!(tz.format_opt(alert.starts_at)));
        obj.insert("ends_at".into(), json!(tz.format_opt(alert.ends_at)));
        obj.insert(
            "generator_url".into(),
            json!(alert.generator_url.clone().unwrap_or_default()),
        );
    }
    view
}

fn annotation(alert: &AlertInstance, key: &str) -> String {
    alert.annotations.get(key).cloned().unwrap_or_default()
}

/* ------------------------------------------------------------------ */
/* 本地构建器指令：循环（repeat）/ 条件（if）                            */
/* ------------------------------------------------------------------ */

/// 循环指令 tag（不是飞书组件；见 `card_catalog` 里 `group = "directive"` 的项）。
pub const DIRECTIVE_REPEAT: &str = "repeat";
/// 条件指令 tag。
pub const DIRECTIVE_IF: &str = "if";

/// 展开卡片里的构建器指令，并渲染所有 `${变量}`。
///
/// 飞书 JSON 2.0 没有循环容器，所以「循环 + 判断 + 文本」是本工具的**本地指令**：
/// 保存的是指令本身，只有真正发送/预览时才在这里展开成飞书认识的组件。
/// 展开后的卡片保证不再包含指令 tag，且**组件数不超过飞书上限**（超出部分丢弃，
/// 与 30KB 体积截断同策略：宁可少显示，也不让整条告警因超限而丢失）。
pub fn expand_card_json(card: &Value, ctx: &RenderContext) -> Value {
    let mut budget = alkh_domain::card_catalog::MAX_CARD_ELEMENTS;
    expand_value(card, ctx, &mut budget)
}

/// 展开一个 JSON 值。`charged` 表示它是否**作为组件**（elements 数组的成员）被计数。
fn expand_value(value: &Value, ctx: &RenderContext, budget: &mut usize) -> Value {
    expand_value_charged(value, ctx, budget, false)
}

fn expand_value_charged(
    value: &Value,
    ctx: &RenderContext,
    budget: &mut usize,
    charged: bool,
) -> Value {
    match value {
        Value::String(text) => Value::String(render_str(text, ctx)),
        Value::Array(items) => Value::Array(expand_list(items, ctx, budget)),
        Value::Object(map) => match map.get("tag").and_then(Value::as_str) {
            Some(DIRECTIVE_REPEAT) => expand_repeat(map, ctx, budget),
            Some(DIRECTIVE_IF) => expand_if(map, ctx, budget),
            Some(_) => {
                // 只有「组件」占名额；header/config 这类结构化对象不计入，也不会被丢弃。
                if charged {
                    if *budget == 0 {
                        return Value::Array(Vec::new());
                    }
                    *budget -= 1;
                }
                Value::Object(
                    map.iter()
                        .map(|(key, item)| {
                            (key.clone(), expand_value_charged(item, ctx, budget, false))
                        })
                        .collect(),
                )
            }
            None => Value::Object(
                map.iter()
                    .map(|(key, item)| {
                        (key.clone(), expand_value_charged(item, ctx, budget, false))
                    })
                    .collect(),
            ),
        },
        other => other.clone(),
    }
}

/// 展开数组：指令可能展开成 0..N 个节点，因此用 flat_map 语义。
fn expand_list(items: &[Value], ctx: &RenderContext, budget: &mut usize) -> Vec<Value> {
    let mut out = Vec::new();
    for item in items {
        match expand_value_charged(item, ctx, budget, true) {
            Value::Array(nested) => out.extend(nested),
            other => out.push(other),
        }
    }
    out
}

/// 循环：对数据源里的每一项渲染一遍 `elements` 模板。
fn expand_repeat(map: &Map<String, Value>, ctx: &RenderContext, budget: &mut usize) -> Value {
    let source = map
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("alerts");
    let items: Vec<Value> = ctx
        .data(source)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let template: Vec<Value> = map
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // 循环次数 = min(数据条数, 策略上限, max_items)；组件数上限由 budget 统一收口。
    let mut limit = items.len();
    if let Some(policy_limit) = ctx.data("alerts_limit").and_then(Value::as_u64) {
        limit = limit.min(policy_limit as usize);
    }
    if let Some(configured) = map.get("max_items").and_then(Value::as_u64) {
        limit = limit.min(configured as usize);
    }
    let had_budget = *budget > 0;

    let mut out: Vec<Value> = Vec::new();
    let mut index = 0;
    while index < limit && index < items.len() && *budget > 0 {
        let scoped = item_context(ctx, &items[index], index + 1);
        let mut expanded = expand_list(&template, &scoped, budget);
        // 同一段模板会被复制 N 份：模板里的 `element_id` 会因此重复，
        // 而飞书对重复 element_id 直接报错（300301）。展开出来的副本一律去 id。
        for node in &mut expanded {
            strip_element_ids(node);
        }
        if expanded.is_empty() && *budget == 0 {
            break; // 名额耗尽
        }
        out.extend(expanded);
        index += 1;
    }

    // 「无数据」兜底文案（相当于循环自带的「判断」）。
    // 只有**一次都没循环**（数据为空、策略上限为 0、或一进来就没名额）时才显示；
    // 循环了但条件全部不成立，不能谎报「无告警」。
    if index == 0 && had_budget {
        if let Some(text) = map.get("empty_content").and_then(Value::as_str) {
            let rendered = render_str(text, ctx);
            if !rendered.trim().is_empty() {
                out.push(json!({ "tag": "markdown", "content": rendered }));
            }
        }
    }
    Value::Array(out)
}

/// 递归删除所有 `element_id`。
///
/// 只用于**循环展开出来的副本**：同一段模板复制多份时，模板里的 id 必然重复，
/// 飞书会以 `300301 duplicate element_id` 拒收整张卡片（=整条告警丢失）。
/// 循环体外（只出现一次）的组件保留自己的 id。
fn strip_element_ids(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("element_id");
            for child in map.values_mut() {
                strip_element_ids(child);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                strip_element_ids(item);
            }
        }
        _ => {}
    }
}

/// 条件：`when` 渲染结果按真值判断；给了 `equals` 则改为精确相等。
fn expand_if(map: &Map<String, Value>, ctx: &RenderContext, budget: &mut usize) -> Value {
    let rendered = map
        .get("when")
        .and_then(Value::as_str)
        .map(|when| render_str(when, ctx))
        .unwrap_or_default();
    let mut truthy = is_truthy(&rendered);
    if let Some(equals) = map.get("equals").and_then(Value::as_str) {
        if !equals.is_empty() {
            truthy = rendered.trim() == render_str(equals, ctx).trim();
        }
    }
    if map.get("negate").and_then(Value::as_bool).unwrap_or(false) {
        truthy = !truthy;
    }
    if !truthy {
        return Value::Array(Vec::new());
    }
    let elements: Vec<Value> = map
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Value::Array(expand_list(&elements, ctx, budget))
}

/// 空串 / false / 0 / no / off 视为「否」，其余非空值为「是」。
fn is_truthy(rendered: &str) -> bool {
    let trimmed = rendered.trim();
    !trimmed.is_empty()
        && !matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        )
}

/// 循环体内的一层上下文：在基线上叠加 `${item.*}` 变量（继承分组级变量）。
fn item_context(base: &RenderContext, item: &Value, index: usize) -> RenderContext {
    let mut ctx = base.clone();
    // 布尔/数字也要能参与 `${...}` 插值与条件判断，统一转成字符串。
    let text = |key: &str| match item.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    ctx.set("item.index", index.to_string());
    for key in [
        "alertname",
        "instance",
        "job",
        "fingerprint",
        "title",
        "status",
        "status_text",
        "status_icon",
        "is_firing",
        "severity",
        "severity_text",
        "severity_color",
        "status_color",
        "summary",
        "description",
        "starts_at",
        "ends_at",
        "generator_url",
    ] {
        ctx.set(format!("item.{key}"), text(key));
    }
    // 与代表告警的 `${alert_status}` 对齐的别名，便于迁移。
    ctx.set("item.alert_status", text("status"));
    // 标签 / 注解：`${item.labels.team}` 与 `${item.label:team}` 两种写法都支持。
    for (prefix, field) in [("label", "labels"), ("annotation", "annotations")] {
        for (key, value) in item
            .get(field)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Some(value) = value.as_str() else {
                continue;
            };
            ctx.set(format!("item.{field}.{key}"), value.to_string());
            ctx.set(format!("item.{prefix}:{key}"), value.to_string());
        }
    }
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

fn alerts_list(group: &AlertGroup, max: usize, tz: DisplayTimeZone) -> String {
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
            lines.push(format!("　‣ 开始：{}", tz.format_opt(a.starts_at)));
        } else {
            lines.push(format!("　‣ 恢复：{}", tz.format_opt(a.ends_at)));
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

    /// 回归：上限远小于内容长度时，`len - over - 后缀` 会**下溢 panic**（debug 构建）。
    #[test]
    fn shrink_never_underflows_when_over_limit_is_large() {
        let mut card = json!({"content": "x".repeat(500)});
        assert!(shrink_json_to_bytes(&mut card, 8));
        assert!(
            serde_json::to_vec(&card).unwrap().len() <= 24,
            "应把内容清掉而不是 panic: {card}"
        );
    }

    /// 循环跑过但条件全部不成立时，**不能**谎报「无告警」。
    #[test]
    fn repeat_does_not_lie_about_empty_when_conditions_filter_everything() {
        let mut ctx = RenderContext::new();
        ctx.set_data("alerts", json!([{"title":"A"},{"title":"B"}]));
        let card = json!({"body":{"elements":[
            {"tag":"repeat","source":"alerts","empty_content":"无告警","elements":[
                {"tag":"if","when":"false","elements":[{"tag":"markdown","content":"never"}]}
            ]}
        ]}});
        assert_eq!(expand_card_json(&card, &ctx)["body"]["elements"], json!([]));
    }

    /// 循环展开后的副本必须去掉 element_id：模板里的 id 会重复 → 飞书 300301。
    #[test]
    fn repeat_strips_template_element_ids_to_avoid_duplicates() {
        let mut ctx = RenderContext::new();
        ctx.set_data("alerts", json!([{"title":"A"},{"title":"B"}]));

        let card = json!({"body":{"elements":[
            {"tag":"markdown","element_id":"header_md","content":"head"},
            {"tag":"repeat","element_id":"loop_1","source":"alerts","elements":[
                {"tag":"markdown","element_id":"item_md","content":"${item.title}"},
                {"tag":"column_set","element_id":"item_cols","columns":[
                    {"tag":"column","elements":[{"tag":"markdown","element_id":"item_inner","content":"x"}]}
                ]}
            ]}
        ]}});

        let out = expand_card_json(&card, &ctx);
        assert_eq!(
            alkh_domain::card_catalog::count_elements(&out),
            1 + 2 * 4,
            "两条告警各展开 4 个组件（markdown + 分栏 + 列 + markdown）"
        );
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("item_md"), "{text}");
        assert!(!text.contains("item_cols"), "{text}");
        assert!(!text.contains("item_inner"), "{text}");
        // 循环体外的 id 保留（只出现一次，是合法的）
        assert!(text.contains("header_md"), "{text}");
        // 展开结果本身必须通过结构校验（无重复 id）
        let report = alkh_domain::card_catalog::validate_card_json(&out);
        assert!(report.is_valid(), "{:?}", report.errors);
    }

    /// 多个循环**共享**同一份 200 组件预算：两个「告警列表」叠加也不会超过飞书上限。
    #[test]
    fn multiple_loops_share_the_card_element_budget() {
        let alerts: Vec<Value> = (0..100)
            .map(|i| json!({"title": format!("A{i}")}))
            .collect();
        let mut ctx = RenderContext::new();
        ctx.set_data("alerts", Value::Array(alerts));

        let loop_node = json!({
            "tag": "repeat", "source": "alerts",
            "elements": [{"tag":"markdown","content":"${item.title}"}]
        });
        let card = json!({"body":{"elements":[
            {"tag":"markdown","content":"head"},
            loop_node.clone(),
            loop_node,
        ]}});

        let out = expand_card_json(&card, &ctx);
        let total = alkh_domain::card_catalog::count_elements(&out);
        assert!(total <= 200, "展开后 {total} 个组件超过飞书上限");
        assert_eq!(out["body"]["elements"][0]["content"], "head");
    }

    #[test]
    fn time_variables_follow_the_display_timezone() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let now = chrono::Utc::now();

        let utc = build_context(&group, &channel, &policy, now, DisplayTimeZone::default());
        let cst = build_context(
            &group,
            &channel,
            &policy,
            now,
            DisplayTimeZone::parse("Asia/Shanghai").unwrap(),
        );

        let utc_first = utc.lookup("first_seen_at").unwrap();
        let cst_first = cst.lookup("first_seen_at").unwrap();
        assert!(utc_first.ends_with("+00:00"), "{utc_first}");
        assert!(cst_first.ends_with("+08:00"), "{cst_first}");
        assert_ne!(utc_first, cst_first);
        assert!(cst.lookup("now").unwrap().ends_with("+08:00"));
        // `data` 通道里的逐条时间同样按展示时区预格式化（循环 `${item.starts_at}`）。
        let item_start = cst
            .data("alerts")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("starts_at"))
            .and_then(|v| v.as_str())
            .unwrap();
        assert!(
            item_start.is_empty() || item_start.ends_with("+08:00"),
            "{item_start}"
        );
    }

    #[test]
    fn context_exposes_documented_variables() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );

        assert_eq!(ctx.lookup("status_text"), Some("未解决"));
        assert_eq!(ctx.lookup("status"), Some("firing"));
        assert_eq!(ctx.lookup("severity_color"), Some("red"));
        assert_eq!(ctx.lookup("status_color"), Some("red"));
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
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );
        assert_eq!(ctx.lookup("instance"), Some("10.0.0.1:9100"));
    }

    #[test]
    fn default_card_renders_without_leftover_placeholders() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );

        let spec = CardMessageSpec::builder_json(crate::defaults::default_card_json());
        let rendered = render_card_message(&spec, &ctx).unwrap();
        let text = serde_json::to_string(&rendered.to_content()).unwrap();

        // 默认卡片引用的所有变量都在上下文中存在（`item.*` 在循环展开时才注入）
        for v in detect_variables_json(&crate::defaults::default_card_json()) {
            if v.starts_with("item.") {
                continue;
            }
            assert!(ctx.lookup(&v).is_some(), "缺少变量 {v}");
        }
        assert!(text.contains("HighCPU"));
        assert!(!text.contains("${"), "渲染后仍残留占位符: {text}");
    }

    /// 结构化「告警列表」：循环体按告警条数展开成真实组件，指令不残留。
    #[test]
    fn structured_loop_expands_one_copy_per_alert() {
        let group = sample_group();
        let channel = sample_channel();
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );

        let spec = CardMessageSpec::builder_json(crate::defaults::default_card_json());
        let RenderedCardMessage::Json(card) = render_card_message(&spec, &ctx).unwrap() else {
            panic!("默认模板必须是本地卡片 JSON");
        };

        let text = serde_json::to_string(&card).unwrap();
        assert!(
            !text.contains("\"repeat\""),
            "渲染后不应残留循环指令: {text}"
        );
        assert!(!text.contains("\"if\""), "渲染后不应残留条件指令: {text}");
        // 两条告警都要出现（实例是逐条区分的字段）
        assert!(text.contains("10.0.0.1:9100"), "{text}");
        assert!(text.contains("10.0.0.2:9100"), "{text}");
        // 条件：firing 那条有 description，resolved 那条没有 → 只应出现一次「详情」
        assert_eq!(text.matches("详情：").count(), 1, "{text}");
        // 条件 + 取反：只有 firing 的那条显示「开始」，只有 resolved 的显示「恢复」
        assert_eq!(text.matches("开始：").count(), 1, "{text}");
        assert_eq!(text.matches("恢复：").count(), 1, "{text}");
    }

    /// 条件指令：假值渲染成空数组，真值才渲染子组件。
    #[test]
    fn if_directive_renders_only_when_truthy() {
        let card = json!({"body":{"elements":[
            {"tag":"if","when":"${flag}","elements":[{"tag":"markdown","content":"shown"}]}
        ]}});

        let yes = RenderContext::new().with("flag", "true");
        assert_eq!(
            expand_card_json(&card, &yes)["body"]["elements"],
            json!([{"tag":"markdown","content":"shown"}])
        );

        for falsy in ["", "false", "0", "off", "NO"] {
            let ctx = RenderContext::new().with("flag", falsy);
            assert_eq!(
                expand_card_json(&card, &ctx)["body"]["elements"],
                json!([]),
                "`{falsy}` 应视为假"
            );
        }
    }

    /// 循环：条数取 min(数据, 策略上限, max_items)，空数据渲染 empty_content。
    #[test]
    fn repeat_respects_limits_and_empty_content() {
        let card = json!({"body":{"elements":[
            {"tag":"repeat","source":"alerts","max_items":1,"empty_content":"无告警",
             "elements":[{"tag":"markdown","content":"${item.title}"}]}
        ]}});

        let mut ctx = RenderContext::new();
        ctx.set_data("alerts", json!([{"title":"A"},{"title":"B"}]));
        assert_eq!(
            expand_card_json(&card, &ctx)["body"]["elements"],
            json!([{"tag":"markdown","content":"A"}])
        );

        let mut limited = RenderContext::new();
        limited.set_data("alerts", json!([{"title":"A"},{"title":"B"}]));
        limited.set_data("alerts_limit", json!(0));
        assert_eq!(
            expand_card_json(&card, &limited)["body"]["elements"],
            json!([{"tag":"markdown","content":"无告警"}])
        );

        let empty = RenderContext::new();
        assert_eq!(
            expand_card_json(&card, &empty)["body"]["elements"],
            json!([{"tag":"markdown","content":"无告警"}])
        );
    }

    /// 循环体里的 `${item.label:x}` / `${item.labels.x}` 都能取到当前告警的标签。
    #[test]
    fn item_scope_exposes_labels_and_annotations() {
        let card = json!({"body":{"elements":[
            {"tag":"repeat","source":"alerts","elements":[
                {"tag":"markdown","content":"${item.index}:${item.label:team}/${item.labels.team}/${item.annotation:runbook}"}
            ]}
        ]}});
        let mut ctx = RenderContext::new();
        ctx.set_data(
            "alerts",
            json!([{"labels":{"team":"sre"},"annotations":{"runbook":"http://rb"}}]),
        );
        assert_eq!(
            expand_card_json(&card, &ctx)["body"]["elements"][0]["content"],
            "1:sre/sre/http://rb"
        );
    }

    /// 展开不能突破飞书 200 组件上限：模板越长，循环次数被压得越少。
    #[test]
    fn repeat_respects_card_element_limit() {
        let alerts: Vec<Value> = (0..100)
            .map(|i| json!({"title": format!("A{i}")}))
            .collect();
        let mut ctx = RenderContext::new();
        ctx.set_data("alerts", Value::Array(alerts));

        let template: Vec<Value> = (0..5)
            .map(|i| json!({"tag":"markdown","content":format!("line {i}: ${{item.title}}")}))
            .collect();
        let card = json!({"body":{"elements":[
            {"tag":"repeat","source":"alerts","elements":template}
        ]}});

        let out = expand_card_json(&card, &ctx);
        let elements = out["body"]["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 200, "展开后恰好压到飞书 200 组件上限");
        assert_eq!(elements[0]["content"], "line 0: A0");
        assert_eq!(elements[199]["content"], "line 4: A39");
    }

    /// 组件上限必须按**实际展开量**算：一个 `if` 展开成 5 行时，
    /// 按模板长度（1）估算会让 100 条告警 × 5 行 = 500 个组件漏过上限。
    #[test]
    fn repeat_element_limit_counts_conditional_children() {
        let alerts: Vec<Value> = (0..100)
            .map(|i| json!({"title": format!("A{i}")}))
            .collect();
        let mut ctx = RenderContext::new();
        ctx.set_data("alerts", Value::Array(alerts));

        let inner: Vec<Value> = (0..5)
            .map(|i| json!({"tag":"markdown","content":format!("row {i}: ${{item.title}}")}))
            .collect();
        let card = json!({"body":{"elements":[
            {"tag":"repeat","source":"alerts","elements":[
                {"tag":"if","when":"true","elements":inner}
            ]}
        ]}});

        let out = expand_card_json(&card, &ctx);
        let elements = out["body"]["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 200, "应截到 200 而不是 500");
        assert_eq!(elements[0]["content"], "row 0: A0");
        assert_eq!(elements[199]["content"], "row 4: A39");
    }

    #[test]
    fn severity_mapping_matches_header_enum() {
        assert_eq!(severity_color(Some("critical")), "red");
        assert_eq!(severity_color(Some("WARNING")), "orange");
        assert_eq!(severity_color(Some("info")), "blue");
        assert_eq!(severity_color(None), "grey");
        assert_eq!(severity_color(Some("nonsense")), "grey");
    }

    /// 回归：未恢复时标题颜色按严重级别；**全部恢复后必须变绿**，
    /// 否则 warning 卡片恢复后仍是 orange，标题颜色不随状态变化。
    #[test]
    fn status_color_turns_green_only_after_full_recovery() {
        assert_eq!(status_color(true, Some("warning")), "orange");
        assert_eq!(status_color(true, Some("critical")), "red");
        assert_eq!(status_color(true, None), "grey");
        assert_eq!(status_color(false, Some("warning")), "green");
        assert_eq!(status_color(false, Some("critical")), "green");
        assert_eq!(status_color(false, None), "green");
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
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );
        let list = ctx.lookup("alerts_list").unwrap();
        assert!(list.contains("另有 **1** 条告警未在此展示"), "{list}");
    }

    fn resolved_warning_group() -> AlertGroup {
        let payload: AlertManagerPayload = serde_json::from_value(serde_json::json!({
            "version":"4","groupKey":"g","status":"resolved","receiver":"r",
            "commonLabels":{"alertname":"X","severity":"warning"},
            "alerts":[{"status":"resolved","labels":{"alertname":"X"},"annotations":{},"fingerprint":"f"}]
        }))
        .unwrap();
        let event = NormalizedEvent::from_payload(&payload).unwrap();
        AlertGroup::new(HookId::new(), ChannelId::new(), &event, chrono::Utc::now())
    }

    #[test]
    fn resolved_group_reports_resolved_status() {
        let group = resolved_warning_group();
        let channel = Channel::new(
            "u",
            "ou_1",
            alkh_domain::channel::ReceiveIdType::OpenId,
            alkh_domain::channel::ChannelKind::P2pUser,
            ChannelSource::Manual,
        );
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );
        assert_eq!(ctx.lookup("status_text"), Some("已恢复"));
        assert_eq!(ctx.lookup("is_firing"), Some("false"));
        // 严重级别本身不变（仍是 warning），但状态色必须变绿
        assert_eq!(ctx.lookup("severity_color"), Some("orange"));
        assert_eq!(ctx.lookup("status_color"), Some("green"));
        assert_eq!(ctx.lookup("status"), Some("resolved"));
    }

    /// 回归：默认模板的标题颜色取自 `${status_color}`，警告恢复后必须由 orange 变 green。
    #[test]
    fn default_card_header_turns_green_after_recovery() {
        let group = resolved_warning_group();
        let channel = Channel::new(
            "u",
            "ou_1",
            alkh_domain::channel::ReceiveIdType::OpenId,
            alkh_domain::channel::ChannelKind::P2pUser,
            ChannelSource::Manual,
        );
        let policy = NotificationPolicy::default_for(channel.id.clone());
        let ctx = build_context(
            &group,
            &channel,
            &policy,
            chrono::Utc::now(),
            DisplayTimeZone::default(),
        );

        let spec = CardMessageSpec::builder_json(crate::defaults::default_card_json());
        let rendered = render_card_message(&spec, &ctx).unwrap();
        let RenderedCardMessage::Json(card) = rendered else {
            panic!("默认模板必须是本地卡片 JSON");
        };
        assert_eq!(
            card["header"]["template"], "green",
            "警告恢复后默认模板标题颜色应变为 green: {card}"
        );
        assert_eq!(
            card["header"]["title"]["content"], "【已恢复】X",
            "标题文案也应显示已恢复: {card}"
        );
    }
}
