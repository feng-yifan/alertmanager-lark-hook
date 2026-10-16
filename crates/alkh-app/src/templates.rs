//! 卡片模板用例：导入、校验、预览、CRUD（需求 1）。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::channel::Channel;
use alkh_domain::ids::CardTemplateId;
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::template::{CardMessageSpec, CardSource, CardTemplate, TemplateVariable};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::{AppError, AppResult};
use crate::meta;

#[derive(Debug, Clone, Deserialize)]
pub struct TemplateInput {
    pub name: String,
    /// 卡片内容；缺省时回退到内置默认卡片（R12 的保存侧对应行为）。
    #[serde(default)]
    pub card_message: Option<CardMessageSpec>,
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
    /// 渲染后的卡片 JSON（与真实投递的内容完全一致）。
    pub rendered_card_json: Value,
    /// 渲染上下文（mock 覆盖后的有效值）。
    pub variables: Value,
    /// 发送给飞书 `content` 字段的内容。
    pub card_content: Value,
}

/// 内置默认模板内容：卡片编辑器用它作为「起步卡片」与旧来源转换的落点，
/// 避免前端另抄一份默认卡片 JSON（会与 `defaults.rs` 漂移）。
#[derive(Debug, Clone, Serialize)]
pub struct DefaultTemplatesView {
    pub card_message: CardMessageSpec,
}

/// 返回内置默认模板内容（不查库）。
pub fn default_templates() -> DefaultTemplatesView {
    DefaultTemplatesView {
        card_message: CardMessageSpec::builder_json(crate::defaults::default_card_json()),
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSample {
    #[default]
    Firing,
    Resolved,
}

impl PreviewSample {
    pub fn is_resolved(self) -> bool {
        matches!(self, PreviewSample::Resolved)
    }
}

pub struct TemplateService {
    deps: Deps,
}

impl TemplateService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    pub async fn list(&self) -> AppResult<Vec<CardTemplateView>> {
        Ok(self
            .deps
            .templates
            .list()
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
        self.enforce(&mut template)?;
        if template.is_default {
            self.deps.templates.clear_default().await?;
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
        self.enforce(&mut template)?;

        if template.is_default && !existing.is_default {
            self.deps.templates.clear_default().await?;
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
        .detail(json!({"name": template.name}));
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
        let mut built = match build(input) {
            Ok(t) => Some(t),
            Err(e) => {
                report.valid = false;
                report.errors.push(e.to_string());
                None
            }
        };

        // 组件目录校验：给出**全部**结构错误，并把可疑用法放进 warnings。
        if let Some(template) = built.as_mut() {
            if let Some(card) = template.card_message.card_json.as_ref() {
                let catalog = alkh_domain::card_catalog::validate_card_json(card);
                if !catalog.errors.is_empty() {
                    report.valid = false;
                }
                report.errors.extend(catalog.errors);
                report.warnings.extend(catalog.warnings);
            }
            // 领域级校验：schema / update_multi / 体积 / 组件目录。
            if let Err(e) = template.validate() {
                let message = e.to_string();
                if !report.errors.contains(&message) {
                    report.valid = false;
                    report.errors.push(message);
                }
            }
        }

        let detected = built.map(|t| t.detected_variables()).unwrap_or_default();
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

    /// 保存前的强制校验（创建 / 更新 / 草稿测试发送共用）。
    ///
    /// 组件目录里的**错误**必须阻断保存：JSON 2.0 对废弃组件与不支持的结构
    /// 会直接报错（如 200861），等到发送时才失败等于告警丢失。
    fn enforce(&self, template: &mut CardTemplate) -> AppResult<()> {
        // P2：停用「卡片搭建工具模板」来源。历史模板仍可读取/预览，
        // 编辑器提供「转换为本地卡片」。
        if template.card_message.source == CardSource::BuilderTemplate {
            return Err(AppError::validation(
                "已停用来源 builder_template：请在卡片编辑器里点「转换为本地卡片」后保存（本地 JSON 卡片同样支持变量与原地更新）",
            ));
        }
        template.validate()?;
        if let Some(card) = template.card_message.card_json.as_ref() {
            let catalog = alkh_domain::card_catalog::validate_card_json(card);
            if let Some(first) = catalog.errors.first() {
                return Err(AppError::validation(format!(
                    "卡片结构不合法：{first}（共 {} 处；详见 /card-templates/validate）",
                    catalog.errors.len()
                )));
            }
        }
        Ok(())
    }

    /// 校验并构造**草稿**模板（不落库），供卡片编辑器「预览 / 测试发送」使用。
    pub fn build_draft(&self, input: TemplateInput) -> AppResult<CardTemplate> {
        let mut template = build(input)?;
        self.enforce(&mut template)?;
        Ok(template)
    }

    /// 返回指定基线（内置样例 / 历史告警分组）渲染出的**全部**变量，
    /// 供卡片编辑器的 mock 面板列出与编辑（P1）。
    pub async fn mock_baseline(
        &self,
        channel_id: Option<alkh_domain::ids::ChannelId>,
        sample: PreviewSample,
        alert_group_id: Option<alkh_domain::ids::AlertGroupId>,
    ) -> AppResult<crate::mock::MockBaseline> {
        let channel = match channel_id {
            Some(id) => self
                .deps
                .channels
                .get(&id)
                .await?
                .ok_or_else(|| AppError::not_found("渠道不存在"))?,
            None => sample_channel(),
        };
        let mock = crate::mock::MockSpec {
            alert_group_id,
            variables: Default::default(),
        };
        crate::mock::mock_baseline(
            &self.deps,
            &channel,
            sample.is_resolved(),
            &mock,
            self.deps.now(),
        )
        .await
    }

    /// 用样例 / mock 数据渲染，供卡片编辑器对照（需求 1 的"导入后可预览"）。
    pub async fn preview(
        &self,
        input: TemplateInput,
        sample: PreviewSample,
        channel_id: Option<alkh_domain::ids::ChannelId>,
        mock: crate::mock::MockSpec,
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

        mock.validate()?;
        let resolved = matches!(sample, PreviewSample::Resolved);
        let group = crate::mock::resolve_mock_group(
            &self.deps,
            &channel,
            alkh_domain::ids::HookId::new(),
            resolved,
            &mock,
            self.deps.now(),
        )
        .await?;
        let mut ctx = crate::render::build_context(
            &group,
            &channel,
            &policy,
            self.deps.now(),
            self.deps.settings.get().await?.display_timezone(),
        );
        mock.apply(&mut ctx);

        let variables = serde_json::to_value(ctx.vars())?;
        let rendered = crate::render::render_card_message(&template.card_message, &ctx)?;
        let content = rendered.to_content();
        Ok(PreviewReport {
            rendered_card_json: content.clone(),
            variables,
            card_content: content,
        })
    }

    async fn audit(&self, actor: &AuditActor, action: &str, t: &CardTemplate) {
        let log = AuditLog::ok(actor.clone(), action, self.deps.now())
            .target(t.id.to_string())
            .detail(json!({"name": t.name}));
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
    }
}

fn build(input: TemplateInput) -> AppResult<CardTemplate> {
    let TemplateInput {
        name,
        card_message,
        is_default,
    } = input;
    let now = alkh_domain::now();
    Ok(CardTemplate {
        id: CardTemplateId::new(),
        name,
        card_message: card_message
            .unwrap_or_else(|| CardMessageSpec::builder_json(crate::defaults::default_card_json())),
        is_default,
        created_at: now,
        updated_at: now,
    })
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
    fn build_falls_back_to_the_builtin_card() {
        let t = build(TemplateInput {
            name: "x".into(),
            card_message: None,
            is_default: false,
        })
        .unwrap();
        assert_eq!(t.card_message.source, CardSource::BuilderJson);
        assert!(t.card_message.card_json.is_some());
    }

    #[test]
    fn template_variables_helper() {
        let v = default_template_variables(&["a".to_string()]);
        assert_eq!(v.len(), 1);
        assert!(v[0].builtin);
    }
}
