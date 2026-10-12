use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{DomainError, DomainResult};
use crate::ids::CardTemplateId;
use crate::Timestamp;

/// 卡片来源：搭建工具导出的 JSON，或搭建工具「发布」后的模板 ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CardSource {
    /// 卡片搭建工具导出的卡片 JSON（CardKit JSON 2.0）。
    #[default]
    BuilderJson,
    /// 卡片搭建工具发布产出的模板（`template_id` + 版本）。
    BuilderTemplate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    #[default]
    CardMessage,
    AppFeedCard,
}

impl TemplateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TemplateKind::CardMessage => "card_message",
            TemplateKind::AppFeedCard => "app_feed_card",
        }
    }
}

/// 应用消息流卡片状态标签类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StatusLabelType {
    #[default]
    Primary,
    Secondary,
    Success,
    Danger,
}

impl StatusLabelType {
    pub fn as_str(self) -> &'static str {
        match self {
            StatusLabelType::Primary => "primary",
            StatusLabelType::Secondary => "secondary",
            StatusLabelType::Success => "success",
            StatusLabelType::Danger => "danger",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeedButtonType {
    #[default]
    Default,
    Primary,
    Success,
}

impl FeedButtonType {
    pub fn as_str(self) -> &'static str {
        match self {
            FeedButtonType::Default => "default",
            FeedButtonType::Primary => "primary",
            FeedButtonType::Success => "success",
        }
    }
}

/// `biz_id` 生成策略：应用消息流卡片以 biz_id 定位并更新。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BizIdSource {
    /// `alkh-{alert_group_id}`（默认，同分组更新同一张卡片）。
    #[default]
    AlertGroup,
    /// `alkh-{hook_id}-{alert_group_id}`。
    Hook,
}

/// 模板变量的类型声明（用于 `template` 来源时按声明类型强转，避免飞书 200732 类型错误）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    #[default]
    String,
    Number,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateVariable {
    pub name: String,
    #[serde(default)]
    pub ty: VariableType,
    #[serde(default)]
    pub default: Option<String>,
    /// 是否来自内置渲染上下文（而非用户自定义）。
    #[serde(default)]
    pub builtin: bool,
}

impl TemplateVariable {
    pub fn builtin(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ty: VariableType::String,
            default: None,
            builtin: true,
        }
    }
    pub fn custom(name: impl Into<String>, ty: VariableType) -> Self {
        Self {
            name: name.into(),
            ty,
            default: None,
            builtin: false,
        }
    }
}

// ---------------------------------------------------------------------------
// 卡片消息模板
// ---------------------------------------------------------------------------

/// 「卡片消息」模板定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardMessageSpec {
    pub source: CardSource,
    /// `source = builder_json`：卡片搭建工具导出的 JSON 2.0。
    #[serde(default)]
    pub card_json: Option<Value>,
    /// `source = builder_template`：搭建工具模板 ID。
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub template_version_name: Option<String>,
    #[serde(default)]
    pub variables: Vec<TemplateVariable>,
}

/// 飞书侧对卡片 JSON 的体积上限（发送卡片消息 30KB）。
pub const CARD_JSON_MAX_BYTES: usize = 30 * 1024;

impl CardMessageSpec {
    pub fn builder_json(card_json: Value) -> Self {
        Self {
            source: CardSource::BuilderJson,
            card_json: Some(card_json),
            template_id: None,
            template_version_name: None,
            variables: Vec::new(),
        }
    }

    pub fn builder_template(template_id: impl Into<String>, version: Option<String>) -> Self {
        Self {
            source: CardSource::BuilderTemplate,
            card_json: None,
            template_id: Some(template_id.into()),
            template_version_name: version,
            variables: Vec::new(),
        }
    }

    /// 校验并归一化（R17：强制 `config.update_multi = true`）。
    pub fn normalize_and_validate(&mut self) -> DomainResult<()> {
        match self.source {
            CardSource::BuilderJson => {
                let card = self.card_json.as_mut().ok_or_else(|| {
                    DomainError::validation("source=builder_json 时必须提供 card_json")
                })?;
                let obj = card
                    .as_object_mut()
                    .ok_or_else(|| DomainError::validation("card_json 必须是 JSON 对象"))?;

                match obj.get("schema").and_then(|v| v.as_str()) {
                    Some("2.0") => {}
                    Some(other) => {
                        return Err(DomainError::validation(format!(
                            "CardKit 仅支持卡片 JSON 2.0 结构，当前 schema = {other}"
                        )))
                    }
                    None => {
                        return Err(DomainError::validation(
                            "card_json 缺少 schema 字段（CardKit 要求 \"schema\": \"2.0\"）",
                        ))
                    }
                }

                // R17：原地更新要求 update_multi 显式为 true。
                let config = obj
                    .entry("config")
                    .or_insert_with(|| Value::Object(Default::default()));
                let config_obj = config
                    .as_object_mut()
                    .ok_or_else(|| DomainError::validation("card_json.config 必须是 JSON 对象"))?;
                if config_obj.get("update_multi").and_then(|v| v.as_bool()) == Some(false) {
                    return Err(DomainError::validation(
                        "card_json.config.update_multi 不能为 false：原地更新（更新旧告警）要求其为 true",
                    ));
                }
                config_obj.insert("update_multi".into(), Value::Bool(true));

                let bytes = serde_json::to_vec(&*card)
                    .map_err(|e| DomainError::validation(format!("card_json 无法序列化: {e}")))?;
                if bytes.len() > CARD_JSON_MAX_BYTES {
                    return Err(DomainError::validation(format!(
                        "card_json 体积 {} 字节，超过飞书卡片消息上限 {} 字节",
                        bytes.len(),
                        CARD_JSON_MAX_BYTES
                    )));
                }
            }
            CardSource::BuilderTemplate => {
                let id = self.template_id.as_deref().unwrap_or_default().trim();
                if id.is_empty() {
                    return Err(DomainError::validation(
                        "source=builder_template 时必须提供 template_id（搭建工具卡片 ID）",
                    ));
                }
                if id.chars().count() > 64 {
                    return Err(DomainError::validation("template_id 过长"));
                }
            }
        }
        for v in &self.variables {
            if v.name.trim().is_empty() {
                return Err(DomainError::validation("模板变量名不能为空"));
            }
        }
        Ok(())
    }

    /// 提取卡片 JSON 中出现的 `${var}` 变量。
    pub fn detected_variables(&self) -> Vec<String> {
        match (&self.source, &self.card_json) {
            (CardSource::BuilderJson, Some(v)) => detect_variables_json(v),
            (CardSource::BuilderTemplate, _) => {
                self.variables.iter().map(|x| x.name.clone()).collect()
            }
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// 应用消息流卡片模板
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusLabelSpec {
    pub text: String,
    pub type_firing: StatusLabelType,
    pub type_resolved: StatusLabelType,
}

impl Default for StatusLabelSpec {
    fn default() -> Self {
        Self {
            text: "${severity_text}".into(),
            type_firing: StatusLabelType::Danger,
            type_resolved: StatusLabelType::Success,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkSpec {
    pub enabled: bool,
    pub url: String,
}

impl Default for LinkSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "${external_url}".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedButtonSpec {
    pub text: String,
    pub url: String,
    pub button_type: FeedButtonType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppFeedCardSpec {
    pub title: String,
    pub preview: String,
    pub status_label: StatusLabelSpec,
    pub link: LinkSpec,
    #[serde(default)]
    pub buttons: Vec<FeedButtonSpec>,
    #[serde(default)]
    pub close_notify: bool,
    #[serde(default)]
    pub biz_id_source: BizIdSource,
}

impl Default for AppFeedCardSpec {
    fn default() -> Self {
        Self {
            title: "【${status_text}】${alertname}".into(),
            preview: "${summary}".into(),
            status_label: StatusLabelSpec::default(),
            link: LinkSpec::default(),
            buttons: vec![FeedButtonSpec {
                text: "查看详情".into(),
                url: "${external_url}".into(),
                button_type: FeedButtonType::Default,
            }],
            close_notify: false,
            biz_id_source: BizIdSource::AlertGroup,
        }
    }
}

impl AppFeedCardSpec {
    pub const MAX_TITLE_CHARS: usize = 60;
    pub const MAX_PREVIEW_CHARS: usize = 120;
    pub const MAX_STATUS_TEXT_CHARS: usize = 20;
    pub const MAX_BUTTONS: usize = 2;
    pub const MAX_BUTTON_TEXT_CHARS: usize = 30;
    pub const MAX_URL_CHARS: usize = 700;

    pub fn validate(&self) -> DomainResult<()> {
        for b in &self.buttons {
            if b.text.trim().is_empty() {
                return Err(DomainError::validation("按钮文案不能为空"));
            }
            if b.url.trim().is_empty() {
                return Err(DomainError::validation("按钮跳转 URL 不能为空"));
            }
        }
        if self.buttons.len() > Self::MAX_BUTTONS {
            return Err(DomainError::validation(format!(
                "应用消息流卡片最多支持 {} 个按钮",
                Self::MAX_BUTTONS
            )));
        }
        Ok(())
    }

    pub fn detected_variables(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in [
            &self.title,
            &self.preview,
            &self.status_label.text,
            &self.link.url,
        ] {
            out.extend(detect_variables(s));
        }
        for b in &self.buttons {
            out.extend(detect_variables(&b.text));
            out.extend(detect_variables(&b.url));
        }
        out.sort();
        out.dedup();
        out
    }
}

/// 渲染后的应用消息流卡片内容（尚未带上 `biz_id` / `time_sensitive`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedAppFeedCard {
    pub title: String,
    pub preview: String,
    pub status_label: Option<RenderedStatusLabel>,
    pub link: Option<String>,
    pub buttons: Vec<RenderedFeedButton>,
    pub close_notify: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedStatusLabel {
    pub text: String,
    pub label_type: StatusLabelType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedFeedButton {
    pub text: String,
    pub url: String,
    pub button_type: FeedButtonType,
}

impl AppFeedCardSpec {
    /// 渲染为飞书 `open_app_feed_card` 的内容（不含 `biz_id` 与 `time_sensitive`）。
    pub fn render(&self, ctx: &RenderContext, is_firing: bool) -> RenderedAppFeedCard {
        let status_text = truncate_chars(
            &render_str(&self.status_label.text, ctx),
            Self::MAX_STATUS_TEXT_CHARS,
        );
        let status_label = if status_text.trim().is_empty() {
            None
        } else {
            Some(RenderedStatusLabel {
                text: status_text,
                label_type: if is_firing {
                    self.status_label.type_firing
                } else {
                    self.status_label.type_resolved
                },
            })
        };

        let link = if self.link.enabled {
            let url = render_str(&self.link.url, ctx);
            if url.trim().is_empty() {
                None
            } else {
                Some(truncate_chars(&url, Self::MAX_URL_CHARS))
            }
        } else {
            None
        };

        let buttons = self
            .buttons
            .iter()
            .take(Self::MAX_BUTTONS)
            .map(|b| RenderedFeedButton {
                text: truncate_chars(&render_str(&b.text, ctx), Self::MAX_BUTTON_TEXT_CHARS),
                url: truncate_chars(&render_str(&b.url, ctx), Self::MAX_URL_CHARS),
                button_type: b.button_type,
            })
            .filter(|b| !b.text.trim().is_empty() && !b.url.trim().is_empty())
            .collect();

        RenderedAppFeedCard {
            title: truncate_chars(&render_str(&self.title, ctx), Self::MAX_TITLE_CHARS),
            preview: truncate_chars(&render_str(&self.preview, ctx), Self::MAX_PREVIEW_CHARS),
            status_label,
            link,
            buttons,
            close_notify: self.close_notify,
        }
    }
}

// ---------------------------------------------------------------------------
// 模板聚合根
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardTemplate {
    pub id: CardTemplateId,
    pub name: String,
    pub kind: TemplateKind,
    /// `kind = card_message` 时非空。
    #[serde(default)]
    pub card_message: Option<CardMessageSpec>,
    /// `kind = app_feed_card` 时非空。
    #[serde(default)]
    pub app_feed_spec: Option<AppFeedCardSpec>,
    pub is_default: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl CardTemplate {
    pub const MAX_NAME_LEN: usize = 64;

    pub fn new_card_message(name: impl Into<String>, spec: CardMessageSpec) -> Self {
        let now = crate::now();
        Self {
            id: CardTemplateId::new(),
            name: name.into(),
            kind: TemplateKind::CardMessage,
            card_message: Some(spec),
            app_feed_spec: None,
            is_default: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn new_app_feed_card(name: impl Into<String>, spec: AppFeedCardSpec) -> Self {
        let now = crate::now();
        Self {
            id: CardTemplateId::new(),
            name: name.into(),
            kind: TemplateKind::AppFeedCard,
            card_message: None,
            app_feed_spec: Some(spec),
            is_default: false,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn validate(&mut self) -> DomainResult<()> {
        if self.name.trim().is_empty() {
            return Err(DomainError::validation("模板名称不能为空"));
        }
        if self.name.chars().count() > Self::MAX_NAME_LEN {
            return Err(DomainError::validation(format!(
                "模板名称不能超过 {} 个字符",
                Self::MAX_NAME_LEN
            )));
        }
        match self.kind {
            TemplateKind::CardMessage => {
                let spec = self.card_message.as_mut().ok_or_else(|| {
                    DomainError::validation("kind=card_message 时必须提供 card_message")
                })?;
                if self.app_feed_spec.is_some() {
                    return Err(DomainError::validation(
                        "kind=card_message 时不应提供 app_feed_spec",
                    ));
                }
                spec.normalize_and_validate()?;
            }
            TemplateKind::AppFeedCard => {
                let spec = self.app_feed_spec.as_ref().ok_or_else(|| {
                    DomainError::validation("kind=app_feed_card 时必须提供 app_feed_spec")
                })?;
                if self.card_message.is_some() {
                    return Err(DomainError::validation(
                        "kind=app_feed_card 时不应提供 card_message",
                    ));
                }
                spec.validate()?;
            }
        }
        Ok(())
    }

    pub fn detected_variables(&self) -> Vec<String> {
        match self.kind {
            TemplateKind::CardMessage => self
                .card_message
                .as_ref()
                .map(|s| s.detected_variables())
                .unwrap_or_default(),
            TemplateKind::AppFeedCard => self
                .app_feed_spec
                .as_ref()
                .map(|s| s.detected_variables())
                .unwrap_or_default(),
        }
    }
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

/// `${var}` / `${label:foo}` / `${annotation:bar}` / `${a.b}` 形式的变量占位符。
pub static VARIABLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{([A-Za-z0-9_.:\-]+)\}").expect("VARIABLE_RE 必须可编译"));

/// 渲染上下文：变量名 → 字符串值。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RenderContext {
    vars: BTreeMap<String, String>,
}

impl RenderContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.set(key, value);
        self
    }

    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    pub fn vars(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    /// 未声明类型时按字符串取值；用于 `template` 来源的变量映射。
    pub fn as_template_variables(
        &self,
        declared: &[TemplateVariable],
    ) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        for v in declared {
            let raw = self
                .lookup(&v.name)
                .map(|s| s.to_string())
                .or_else(|| v.default.clone())
                .unwrap_or_default();
            let json = match v.ty {
                VariableType::String => Value::String(raw),
                VariableType::Number => raw
                    .parse::<f64>()
                    .map(|n| {
                        serde_json::Number::from_f64(n)
                            .map(Value::Number)
                            .unwrap_or(Value::Null)
                    })
                    .unwrap_or(Value::Null),
                VariableType::Bool => Value::Bool(matches!(raw.as_str(), "true" | "1" | "yes")),
            };
            map.insert(v.name.clone(), json);
        }
        map
    }
}

/// 渲染字符串，未知变量替换为空串。
pub fn render_str(template: &str, ctx: &RenderContext) -> String {
    render_str_tracked(template, ctx).0
}

/// 渲染字符串，同时返回未解析到的变量名（用于校验提示）。
pub fn render_str_tracked(template: &str, ctx: &RenderContext) -> (String, Vec<String>) {
    let mut missing = Vec::new();
    let rendered = VARIABLE_RE
        .replace_all(template, |caps: &regex::Captures<'_>| {
            let key = &caps[1];
            match ctx.lookup(key) {
                Some(v) => v.to_string(),
                None => {
                    missing.push(key.to_string());
                    String::new()
                }
            }
        })
        .into_owned();
    (rendered, missing)
}

/// 递归渲染 JSON 的所有字符串叶子节点（不改变结构）。
pub fn render_json(value: &Value, ctx: &RenderContext) -> Value {
    match value {
        Value::String(s) => Value::String(render_str(s, ctx)),
        Value::Array(items) => Value::Array(items.iter().map(|v| render_json(v, ctx)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), render_json(v, ctx)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// 提取字符串中的所有变量名（去重排序）。
pub fn detect_variables(template: &str) -> Vec<String> {
    let mut out: Vec<String> = VARIABLE_RE
        .captures_iter(template)
        .map(|c| c[1].to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 提取 JSON 中所有字符串叶子节点里的变量名（去重排序）。
pub fn detect_variables_json(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_variables(value, &mut out);
    out.sort();
    out.dedup();
    out
}

fn collect_variables(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.extend(detect_variables(s)),
        Value::Array(items) => items.iter().for_each(|v| collect_variables(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_variables(v, out)),
        _ => {}
    }
}

/// 按**字符**（而非字节）截断，避免截坏 UTF-8。
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn card_json_requires_schema_2() {
        let mut spec = CardMessageSpec::builder_json(json!({"elements": []}));
        assert!(spec.normalize_and_validate().is_err());

        let mut spec = CardMessageSpec::builder_json(json!({"schema": "1.0"}));
        assert!(spec.normalize_and_validate().is_err());
    }

    #[test]
    fn update_multi_is_forced_true() {
        let mut spec = CardMessageSpec::builder_json(json!({
            "schema": "2.0",
            "body": {"elements": []}
        }));
        spec.normalize_and_validate().unwrap();
        assert_eq!(
            spec.card_json.as_ref().unwrap()["config"]["update_multi"],
            json!(true)
        );
    }

    #[test]
    fn update_multi_false_is_rejected() {
        let mut spec = CardMessageSpec::builder_json(json!({
            "schema": "2.0",
            "config": {"update_multi": false},
            "body": {"elements": []}
        }));
        assert!(spec.normalize_and_validate().is_err());
    }

    #[test]
    fn renders_variables_and_reports_missing() {
        let ctx = RenderContext::new()
            .with("alertname", "HighCPU")
            .with("status_text", "未解决");
        let (out, missing) = render_str_tracked("${alertname} / ${status_text} / ${nope}", &ctx);
        assert_eq!(out, "HighCPU / 未解决 / ");
        assert_eq!(missing, vec!["nope".to_string()]);
    }

    #[test]
    fn render_json_only_touches_string_leaves() {
        let ctx = RenderContext::new().with("x", "1");
        let input = json!({"a": "${x}", "b": 3, "c": ["${x}", true]});
        let out = render_json(&input, &ctx);
        assert_eq!(out, json!({"a": "1", "b": 3, "c": ["1", true]}));
    }

    #[test]
    fn detects_variables_in_json() {
        let v = json!({"header": {"title": {"content": "${alertname} ${severity}"}},
                       "body": {"elements": [{"content": "${alerts_table}"}]}});
        assert_eq!(
            detect_variables_json(&v),
            vec![
                "alertname".to_string(),
                "alerts_table".into(),
                "severity".into()
            ]
        );
    }

    #[test]
    fn app_feed_render_truncates_to_feishu_limits() {
        let spec = AppFeedCardSpec {
            title: "${long}".into(),
            preview: "${long}".into(),
            ..Default::default()
        };
        let ctx = RenderContext::new().with("long", "字".repeat(300));
        let r = spec.render(&ctx, true);
        assert_eq!(r.title.chars().count(), AppFeedCardSpec::MAX_TITLE_CHARS);
        assert_eq!(
            r.preview.chars().count(),
            AppFeedCardSpec::MAX_PREVIEW_CHARS
        );
    }

    #[test]
    fn template_variables_are_coerced_by_declared_type() {
        let ctx = RenderContext::new()
            .with("n", "42")
            .with("b", "true")
            .with("s", "hi");
        let declared = vec![
            TemplateVariable::custom("n", VariableType::Number),
            TemplateVariable::custom("b", VariableType::Bool),
            TemplateVariable::custom("s", VariableType::String),
        ];
        let map = ctx.as_template_variables(&declared);
        assert!(map["n"].is_number());
        assert_eq!(map["b"], json!(true));
        assert_eq!(map["s"], json!("hi"));
    }

    #[test]
    fn template_source_requires_template_id() {
        let mut spec = CardMessageSpec::builder_template("", None);
        assert!(spec.normalize_and_validate().is_err());
        let mut spec = CardMessageSpec::builder_template("AAq123", Some("1.0.0".into()));
        assert!(spec.normalize_and_validate().is_ok());
    }
}
