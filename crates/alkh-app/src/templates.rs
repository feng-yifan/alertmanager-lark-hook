//! 卡片模板用例：导入、校验、预览、CRUD（需求 1）。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::channel::Channel;
use alkh_domain::ids::CardTemplateId;
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::template::{
    AppFeedCardSpec, CardMessageSpec, CardTemplate, TemplateKind, TemplateVariable,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::{AppError, AppResult};
use crate::meta;

#[derive(Debug, Clone, Deserialize)]
pub struct TemplateInput {
    pub name: String,
    pub kind: TemplateKind,
    #[serde(default)]
    pub card_message: Option<CardMessageSpec>,
    #[serde(default)]
    pub app_feed_spec: Option<AppFeedCardSpec>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardTemplateView {
    #[serde(flatten)]
    pub template: CardTemplate,
    pub detected_variables: Vec<String>,
    pub unknown_variables: Vec<String>,
}

impl CardTemplateView {
    fn of(t: CardTemplate) -> Self {
        let detected = t.detected_variables();
        let unknown = unknown_variables(&detected);
        Self {
            template: t,
            detected_variables: detected,
            unknown_variables: unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub detected_variables: Vec<String>,
    pub unknown_variables: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewReport {
    pub kind: TemplateKind,
    pub rendered_card_json: Option<Value>,
    pub rendered_app_feed_spec: Option<Value>,
    pub variables: Value,
    pub card_content: Value,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSample {
    #[default]
    Firing,
    Resolved,
}

pub struct TemplateService {
    deps: Deps,
}

impl TemplateService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    pub async fn list(&self, kind: Option<TemplateKind>) -> AppResult<Vec<CardTemplateView>> {
        Ok(self
            .deps
            .templates
            .list(kind)
            .await?
            .into_iter()
            .map(CardTemplateView::of)
            .collect())
    }

    pub async fn get(&self, id: &CardTemplateId) -> AppResult<CardTemplateView> {
        let t = self
            .deps
            .templates
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("卡片模板不存在"))?;
        Ok(CardTemplateView::of(t))
    }

    pub async fn create(
        &self,
        input: TemplateInput,
        actor: &AuditActor,
    ) -> AppResult<CardTemplateView> {
        let mut template = build(input)?;
        template.validate()?;
        if template.is_default {
            self.deps.templates.clear_default(template.kind).await?;
        }
        self.deps.templates.insert(&template).await?;
        self.audit(actor, audit_action::TEMPLATE_CREATED, &template)
            .await;
        Ok(CardTemplateView::of(template))
    }

    pub async fn update(
        &self,
        id: &CardTemplateId,
        input: TemplateInput,
        actor: &AuditActor,
    ) -> AppResult<CardTemplateView> {
        let existing = self
            .deps
            .templates
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("卡片模板不存在"))?;
        let mut template = build(input)?;
        template.id = existing.id.clone();
        template.created_at = existing.created_at;
        template.updated_at = self.deps.now();
        template.validate()?;

        if template.is_default && !existing.is_default {
            self.deps.templates.clear_default(template.kind).await?;
        }
        self.deps.templates.update(&template).await?;
        self.audit(actor, audit_action::TEMPLATE_UPDATED, &template)
            .await;
        Ok(CardTemplateView::of(template))
    }

    pub async fn delete(&self, id: &CardTemplateId, actor: &AuditActor) -> AppResult<()> {
        let template = self
            .deps
            .templates
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("卡片模板不存在"))?;
        let used = self.deps.policies.count_by_template(id).await?;
        if used > 0 {
            return Err(AppError::conflict(format!(
                "模板「{}」正被 {used} 个渠道策略引用，无法删除",
                template.name
            )));
        }
        self.deps.templates.delete(id).await?;
        let log = AuditLog::ok(
            actor.clone(),
            audit_action::TEMPLATE_DELETED,
            self.deps.now(),
        )
        .target(template.id.to_string())
        .detail(json!({"name": template.name, "kind": template.kind.as_str()}));
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
        Ok(())
    }

    /// 仅校验，不落库。
    pub fn validate(&self, input: TemplateInput) -> ValidationReport {
        let mut report = ValidationReport {
            valid: true,
            ..Default::default()
        };
        let detected = match build(input) {
            Ok(mut t) => {
                if let Err(e) = t.validate() {
                    report.valid = false;
                    report.errors.push(e.to_string());
                }
                t.detected_variables()
            }
            Err(e) => {
                report.valid = false;
                report.errors.push(e.to_string());
                Vec::new()
            }
        };
        report.unknown_variables = unknown_variables(&detected);
        if !report.unknown_variables.is_empty() {
            report.warnings.push(format!(
                "以下变量不在内置清单中，渲染时会替换为空串：{}",
                report.unknown_variables.join(", ")
            ));
        }
        report.detected_variables = detected;
        report
    }

    /// 用样例数据渲染，供搭建工具对照（需求 1 的"导入后可预览"）。
    pub async fn preview(
        &self,
        input: TemplateInput,
        sample: PreviewSample,
        channel_id: Option<alkh_domain::ids::ChannelId>,
    ) -> AppResult<PreviewReport> {
        let mut template = build(input)?;
        template.validate()?;

        let (channel, policy) = match channel_id {
            Some(id) => {
                let c = self
                    .deps
                    .channels
                    .get(&id)
                    .await?
                    .ok_or_else(|| AppError::not_found("渠道不存在"))?;
                let p = self
                    .deps
                    .policies
                    .get(&id)
                    .await?
                    .unwrap_or_else(|| NotificationPolicy::default_for(id.clone()));
                (c, p)
            }
            None => (
                sample_channel(),
                NotificationPolicy::default_for(Default::default()),
            ),
        };

        let resolved = matches!(sample, PreviewSample::Resolved);
        let group = crate::query::sample_alert_group(
            &channel,
            alkh_domain::ids::HookId::new(),
            resolved,
            self.deps.now(),
        );
        let ctx = crate::render::build_context(&group, &channel, &policy, self.deps.now());
        let is_firing = crate::render::is_firing_group(&group);

        let variables = serde_json::to_value(ctx.vars())?;
        match template.kind {
            TemplateKind::CardMessage => {
                let spec = template.card_message.clone().unwrap();
                let rendered = crate::render::render_card_message(&spec, &ctx)?;
                Ok(PreviewReport {
                    kind: template.kind,
                    rendered_card_json: Some(rendered.to_content()),
                    rendered_app_feed_spec: None,
                    variables,
                    card_content: rendered.to_content(),
                })
            }
            TemplateKind::AppFeedCard => {
                let spec = template.app_feed_spec.clone().unwrap();
                let rendered = crate::render::render_app_feed_card(&spec, &ctx, is_firing);
                Ok(PreviewReport {
                    kind: template.kind,
                    rendered_card_json: None,
                    rendered_app_feed_spec: Some(serde_json::to_value(&rendered)?),
                    variables,
                    card_content: serde_json::to_value(&rendered)?,
                })
            }
        }
    }

    async fn audit(&self, actor: &AuditActor, action: &str, t: &CardTemplate) {
        let log = AuditLog::ok(actor.clone(), action, self.deps.now())
            .target(t.id.to_string())
            .detail(json!({"name": t.name, "kind": t.kind.as_str()}));
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
    }
}

fn build(input: TemplateInput) -> AppResult<CardTemplate> {
    let TemplateInput {
        name,
        kind,
        card_message,
        app_feed_spec,
        is_default,
    } = input;
    let now = alkh_domain::now();
    let template = match kind {
        TemplateKind::CardMessage => CardTemplate {
            id: CardTemplateId::new(),
            name,
            kind,
            card_message: Some(card_message.unwrap_or_else(|| {
                CardMessageSpec::builder_json(crate::defaults::default_card_json())
            })),
            app_feed_spec: None,
            is_default,
            created_at: now,
            updated_at: now,
        },
        TemplateKind::AppFeedCard => CardTemplate {
            id: CardTemplateId::new(),
            name,
            kind,
            card_message: None,
            app_feed_spec: Some(app_feed_spec.unwrap_or_default()),
            is_default,
            created_at: now,
            updated_at: now,
        },
    };
    Ok(template)
}

fn unknown_variables(detected: &[String]) -> Vec<String> {
    let known = meta::render_variable_names();
    detected
        .iter()
        .filter(|v| !known.contains(v) && !v.contains(':'))
        .cloned()
        .collect()
}

fn sample_channel() -> Channel {
    Channel::from_chat("oc_preview", "预览渠道", None, false)
}

/// 声明模板变量（供 UI 在 `builder_template` 模式下编辑类型）。
pub fn default_template_variables(names: &[String]) -> Vec<TemplateVariable> {
    names
        .iter()
        .map(|n| TemplateVariable::builtin(n.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_variables_detected() {
        let detected = vec![
            "alertname".to_string(),
            "not_a_var".to_string(),
            "label:team".to_string(),
        ];
        let unknown = unknown_variables(&detected);
        assert_eq!(unknown, vec!["not_a_var".to_string()]);
    }

    #[test]
    fn build_defaults_by_kind() {
        let t = build(TemplateInput {
            name: "x".into(),
            kind: TemplateKind::CardMessage,
            card_message: None,
            app_feed_spec: None,
            is_default: false,
        })
        .unwrap();
        assert!(t.card_message.is_some());
        assert!(t.app_feed_spec.is_none());

        let t = build(TemplateInput {
            name: "y".into(),
            kind: TemplateKind::AppFeedCard,
            card_message: None,
            app_feed_spec: None,
            is_default: false,
        })
        .unwrap();
        assert!(t.app_feed_spec.is_some());
        assert!(t.card_message.is_none());
    }

    #[test]
    fn template_variables_helper() {
        let v = default_template_variables(&["a".to_string()]);
        assert_eq!(v.len(), 1);
        assert!(v[0].builtin);
    }
}
