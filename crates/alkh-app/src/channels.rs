//! 渠道用例：CRUD、从飞书同步、测试消息。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::channel::{Channel, ChannelKind, ChannelSource, ReceiveIdType};
use alkh_domain::ids::{ChannelId, HookId};
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::ports::{ChannelFilter, DeliveryFilter};
use alkh_domain::settings::AppSettings;
use alkh_domain::{Page, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::deps::Deps;
use crate::error::{AppError, AppResult};
use crate::sender::NotificationSender;

#[derive(Debug, Clone, Serialize)]
pub struct HookView {
    pub id: HookId,
    pub name: String,
    pub token_prefix: String,
    /// 屏蔽 token 的路径模板，供 UI 展示"该往哪里配"。
    pub hook_path: String,
    pub hook_url: String,
    pub enabled: bool,
    pub last_called_at: Option<Timestamp>,
    pub call_count: u64,
    pub created_at: Timestamp,
    pub rotated_at: Option<Timestamp>,
}

impl HookView {
    pub fn from_hook(h: &alkh_domain::hook::Hook, settings: &AppSettings) -> Self {
        let masked = format!("{}…", h.token_prefix);
        Self {
            id: h.id.clone(),
            name: h.name.clone(),
            token_prefix: h.token_prefix.clone(),
            hook_path: format!("/api/v1/hooks/alertmanager/{masked}"),
            hook_url: settings.hook_url(&masked),
            enabled: h.enabled,
            last_called_at: h.last_called_at,
            call_count: h.call_count,
            created_at: h.created_at,
            rotated_at: h.rotated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelSummary {
    pub id: ChannelId,
    pub name: String,
    pub receive_id: String,
    pub receive_id_type: ReceiveIdType,
    pub kind: ChannelKind,
    pub avatar_url: Option<String>,
    pub external: bool,
    pub enabled: bool,
    pub bot_can_send: bool,
    pub source: ChannelSource,
    pub hook_count: usize,
    pub policy: NotificationPolicy,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelDetail {
    #[serde(flatten)]
    pub summary: ChannelSummary,
    pub hooks: Vec<HookView>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub receive_id: String,
    pub receive_id_type: ReceiveIdType,
    #[serde(default)]
    pub kind: Option<ChannelKind>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateChannel {
    pub name: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SyncRequest {
    #[serde(default = "default_true")]
    pub create_missing: bool,
    #[serde(default = "default_true")]
    pub import_names: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SyncReport {
    pub discovered: usize,
    pub created: usize,
    pub updated: usize,
    pub disabled: usize,
    pub skipped: usize,
}

pub struct ChannelService {
    deps: Deps,
}

impl ChannelService {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    pub async fn list(&self, filter: ChannelFilter) -> AppResult<Page<ChannelSummary>> {
        let settings = self.deps.settings.get().await?;
        let page = self.deps.channels.list(filter).await?;
        let mut items = Vec::with_capacity(page.items.len());
        for c in &page.items {
            items.push(self.summary(c, &settings).await?);
        }
        Ok(Page::new(items, page.total, page.limit, page.offset))
    }

    pub async fn get(&self, id: &ChannelId) -> AppResult<ChannelDetail> {
        let settings = self.deps.settings.get().await?;
        let channel = self
            .deps
            .channels
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在"))?;
        let summary = self.summary(&channel, &settings).await?;
        let hooks = self
            .deps
            .hooks
            .list(Some(id))
            .await?
            .iter()
            .map(|h| HookView::from_hook(h, &settings))
            .collect();
        Ok(ChannelDetail { summary, hooks })
    }

    async fn summary(
        &self,
        channel: &Channel,
        settings: &AppSettings,
    ) -> AppResult<ChannelSummary> {
        let hooks = self.deps.hooks.list(Some(&channel.id)).await?;
        let policy = self
            .deps
            .policies
            .get(&channel.id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(channel.id.clone()));
        let _ = settings;
        Ok(ChannelSummary {
            id: channel.id.clone(),
            name: channel.name.clone(),
            receive_id: channel.receive_id.clone(),
            receive_id_type: channel.receive_id_type,
            kind: channel.kind,
            avatar_url: channel.avatar_url.clone(),
            external: channel.external,
            enabled: channel.enabled,
            bot_can_send: channel.bot_can_send,
            source: channel.source,
            hook_count: hooks.len(),
            policy,
            created_at: channel.created_at,
            updated_at: channel.updated_at,
        })
    }

    /// 手工创建渠道：同时创建默认策略与一个默认 Hook。
    /// 返回 `(详情, 明文 hook token)`——token 只在此刻返回一次。
    pub async fn create(
        &self,
        input: CreateChannel,
        actor: &AuditActor,
    ) -> AppResult<(ChannelDetail, String)> {
        let rt = input.receive_id_type;
        let kind = input.kind.unwrap_or(match rt {
            ReceiveIdType::ChatId => ChannelKind::Group,
            _ => ChannelKind::P2pUser,
        });
        let mut channel = Channel::new(
            input.name,
            input.receive_id,
            rt,
            kind,
            ChannelSource::Manual,
        );
        channel.enabled = input.enabled.unwrap_or(true);
        channel.validate()?;

        if self
            .deps
            .channels
            .find_by_receive_id(channel.receive_id_type, &channel.receive_id)
            .await?
            .is_some()
        {
            return Err(AppError::conflict("该接收者已存在对应渠道"));
        }

        self.deps.channels.upsert(&channel).await?;
        let policy = NotificationPolicy::default_for(channel.id.clone());
        self.deps.policies.upsert(&policy).await?;

        let (hook, token) = alkh_domain::hook::Hook::issue(channel.id.clone(), "default");
        hook.validate()?;
        self.deps.hooks.insert(&hook).await?;

        self.audit(
            actor,
            audit_action::CHANNEL_CREATED,
            channel.id.as_ref(),
            json!({"receive_id_type": channel.receive_id_type.as_str()}),
        )
        .await;

        Ok((self.get(&channel.id).await?, token))
    }

    pub async fn update(
        &self,
        id: &ChannelId,
        input: UpdateChannel,
        actor: &AuditActor,
    ) -> AppResult<ChannelDetail> {
        let mut channel = self
            .deps
            .channels
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在"))?;
        if let Some(name) = input.name {
            channel.name = name;
        }
        if let Some(enabled) = input.enabled {
            channel.enabled = enabled;
        }
        channel.updated_at = self.deps.now();
        channel.validate()?;
        self.deps.channels.upsert(&channel).await?;

        self.audit(
            actor,
            audit_action::CHANNEL_UPDATED,
            channel.id.as_ref(),
            json!({"enabled": channel.enabled, "name": channel.name}),
        )
        .await;
        self.get(id).await
    }

    /// 删除渠道（级联 hooks 与策略）。存在投递历史时需要 `force`。
    pub async fn delete(&self, id: &ChannelId, force: bool, actor: &AuditActor) -> AppResult<()> {
        let channel = self
            .deps
            .channels
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在"))?;

        let deliveries = self
            .deps
            .deliveries
            .list(DeliveryFilter {
                channel_id: Some(id.clone()),
                limit: 1,
                ..Default::default()
            })
            .await?;
        if deliveries.total > 0 && !force {
            return Err(AppError::conflict(
                "该渠道存在投递历史，如需删除请使用 force=true（历史记录会保留）",
            ));
        }

        for h in self.deps.hooks.list(Some(id)).await? {
            self.deps.hooks.delete(&h.id).await?;
        }
        self.deps.policies.delete_for_channel(id).await?;
        self.deps.channels.delete(id).await?;

        self.audit(
            actor,
            audit_action::CHANNEL_DELETED,
            channel.id.as_ref(),
            json!({"name": channel.name, "force": force}),
        )
        .await;
        Ok(())
    }

    /// 从飞书同步机器人所在群列表（需求 3）。
    pub async fn sync(&self, req: SyncRequest, actor: &AuditActor) -> AppResult<SyncReport> {
        let chats = self.deps.lark.list_chats().await?;
        let mut report = SyncReport {
            discovered: chats.len(),
            ..Default::default()
        };
        let now = self.deps.now();

        for chat in &chats {
            if chat.name.trim().is_empty() && !req.create_missing {
                report.skipped += 1;
                continue;
            }
            match self
                .deps
                .channels
                .find_by_receive_id(ReceiveIdType::ChatId, &chat.chat_id)
                .await?
            {
                Some(existing) => {
                    let mut c = existing;
                    let mut changed = false;
                    if req.import_names && !chat.name.trim().is_empty() && c.name != chat.name {
                        c.name = chat.name.clone();
                        changed = true;
                    }
                    if c.avatar_url != chat.avatar_url {
                        c.avatar_url = chat.avatar_url.clone();
                        changed = true;
                    }
                    if c.external != chat.external {
                        c.external = chat.external;
                        changed = true;
                    }
                    // 群已解散 ⇒ 自动停用
                    if chat.dissolved && c.enabled {
                        c.enabled = false;
                        changed = true;
                        report.disabled += 1;
                    }
                    if c.bot_can_send == chat.dissolved {
                        c.bot_can_send = !chat.dissolved;
                        changed = true;
                    }
                    if changed {
                        c.updated_at = now;
                        self.deps.channels.upsert(&c).await?;
                        report.updated += 1;
                    } else {
                        report.skipped += 1;
                    }
                }
                None if req.create_missing => {
                    let mut c = Channel::from_chat(
                        chat.chat_id.clone(),
                        if chat.name.trim().is_empty() {
                            format!("飞书群 {}", &chat.chat_id[..chat.chat_id.len().min(12)])
                        } else {
                            chat.name.clone()
                        },
                        chat.avatar_url.clone(),
                        chat.external,
                    );
                    c.bot_can_send = !chat.dissolved;
                    c.enabled = !chat.dissolved;
                    c.validate()?;
                    self.deps.channels.upsert(&c).await?;
                    self.deps
                        .policies
                        .upsert(&NotificationPolicy::default_for(c.id.clone()))
                        .await?;
                    let (hook, _token) = alkh_domain::hook::Hook::issue(c.id.clone(), "default");
                    self.deps.hooks.insert(&hook).await?;
                    report.created += 1;
                }
                None => report.skipped += 1,
            }
        }

        self.audit(
            actor,
            audit_action::CHANNEL_SYNCED,
            "-",
            json!({
                "discovered": report.discovered,
                "created": report.created,
                "updated": report.updated,
                "disabled": report.disabled,
            }),
        )
        .await;
        Ok(report)
    }

    /// 向该渠道发送一条测试消息。
    pub async fn send_test(
        &self,
        id: &ChannelId,
        card_template_id: Option<alkh_domain::ids::CardTemplateId>,
        sample: TestSample,
        mock: &crate::mock::MockSpec,
        actor: &AuditActor,
    ) -> AppResult<alkh_domain::alert::Delivery> {
        self.send_test_inner(id, card_template_id, None, sample, mock, actor)
            .await
    }

    /// 用**未保存的草稿模板** + mock 数据发送一条测试消息（管理端卡片编辑器 P1）。
    ///
    /// 草稿不会落库，也不写进 `Delivery.card_template_id`；消息本身仍归属该渠道的
    /// 「测试分组」，因此测试消息依旧可更新 / 可撤回。
    pub async fn send_test_draft(
        &self,
        id: &ChannelId,
        template: &alkh_domain::template::CardTemplate,
        sample: TestSample,
        mock: &crate::mock::MockSpec,
        actor: &AuditActor,
    ) -> AppResult<alkh_domain::alert::Delivery> {
        self.send_test_inner(id, None, Some(template), sample, mock, actor)
            .await
    }

    async fn send_test_inner(
        &self,
        id: &ChannelId,
        card_template_id: Option<alkh_domain::ids::CardTemplateId>,
        draft: Option<&alkh_domain::template::CardTemplate>,
        sample: TestSample,
        mock: &crate::mock::MockSpec,
        actor: &AuditActor,
    ) -> AppResult<alkh_domain::alert::Delivery> {
        mock.validate()?;
        let channel = self
            .deps
            .channels
            .get(id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在"))?;
        let mut policy = self
            .deps
            .policies
            .get(id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(id.clone()));
        if card_template_id.is_some() {
            policy.card_template_id = card_template_id;
        }
        // 草稿由调用方显式提供，策略里的模板引用不应参与解析。
        if draft.is_some() {
            policy.card_template_id = None;
        }
        policy.validate(&channel)?;

        // 校验模板存在
        if let Some(tid) = &policy.card_template_id {
            self.deps
                .templates
                .get(tid)
                .await?
                .ok_or_else(|| AppError::not_found("卡片模板不存在"))?;
        }

        // 复用渠道的第一个 Hook，使同一渠道的多次测试复用同一个「测试分组」行，
        // 这样测试投递同样具有可更新/可撤回的告警分组归属（数据自洽）。
        let hook_id = self
            .deps
            .hooks
            .list(Some(&channel.id))
            .await?
            .first()
            .map(|h| h.id.clone())
            .unwrap_or_default();

        // mock 数据源分组：历史分组优先，否则就是内置样例。
        // 历史分组只作为**渲染数据源**，绝不回写（见 `crate::mock` 模块说明）。
        let mock_source = crate::mock::resolve_mock_group(
            &self.deps,
            &channel,
            hook_id.clone(),
            sample.is_resolved(),
            mock,
            self.deps.now(),
        )
        .await?;
        // 选了历史分组就以它的状态为准，保证卡片里的状态与数据一致。
        let resolved = if mock.alert_group_id.is_some() {
            mock_source.status == alkh_domain::alert::GroupStatus::Resolved
        } else {
            sample.is_resolved()
        };
        let group = if mock.alert_group_id.is_some() {
            crate::query::sample_alert_group(&channel, hook_id, resolved, self.deps.now())
        } else {
            mock_source.clone()
        };
        // 必须在发送前落库：Delivery 引用该分组，撤回/重试都需要它存在。
        self.deps.groups.upsert(&group).await?;

        let mut ctx =
            crate::render::build_context(&mock_source, &channel, &policy, self.deps.now());
        mock.apply(&mut ctx);

        let sender = NotificationSender::new(self.deps.clone());
        let prepared = sender
            .prepare_with_context(&group, &channel, &policy, draft, Some(&ctx))
            .await?;
        let result = sender
            .deliver(
                &prepared,
                alkh_domain::alert::DeliveryKind::Test,
                alkh_domain::alert::DeliveryAction::SendNew,
                None,
                None,
            )
            .await?;

        self.audit(
            actor,
            audit_action::TEST_MESSAGE,
            channel.id.as_ref(),
            json!({
                "delivery_id": result.current.id.to_string(),
                "state": format!("{:?}", result.current.state),
                "draft": draft.map(|t| t.name.clone()),
                "mock_variables": mock.variables.len(),
                "mock_alert_group_id": mock.alert_group_id.as_ref().map(|g| g.to_string()),
            }),
        )
        .await;
        Ok(result.current)
    }

    async fn audit(
        &self,
        actor: &AuditActor,
        action: &str,
        target: &str,
        detail: serde_json::Value,
    ) {
        let log = AuditLog::ok(actor.clone(), action, self.deps.now())
            .target(target)
            .detail(detail);
        if let Err(e) = self.deps.audit.append(&log).await {
            tracing::warn!(error = %e, "审计写入失败");
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestSample {
    #[default]
    Firing,
    Resolved,
}

impl TestSample {
    pub fn is_resolved(self) -> bool {
        matches!(self, TestSample::Resolved)
    }
}
