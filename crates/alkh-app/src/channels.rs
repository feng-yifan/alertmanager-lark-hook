//! 渠道用例：CRUD、从飞书同步、测试消息。

use alkh_domain::audit::{action as audit_action, AuditActor, AuditLog};
use alkh_domain::channel::{Channel, ChannelKind, ChannelSource, ReceiveIdType};
use alkh_domain::ids::{ChannelId, HookId};
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::ports::{ChannelFilter, DeliveryFilter, SecretCipher};
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
    /// 完整 Hook 路径（含 token）；旧数据无法回显时退化为遮蔽形式（含 `…`）。
    pub hook_path: String,
    /// 完整 Hook 地址：由「设置 → 对外访问地址」拼出，**与浏览器当前地址无关**。
    pub hook_url: String,
    /// token 是否已回显。旧版本（0003 迁移前）创建的 Hook 为 `false`，轮换一次即可恢复完整地址。
    pub hook_url_available: bool,
    pub enabled: bool,
    pub last_called_at: Option<Timestamp>,
    pub call_count: u64,
    pub created_at: Timestamp,
    pub rotated_at: Option<Timestamp>,
}

impl HookView {
    pub fn from_hook(
        h: &alkh_domain::hook::Hook,
        settings: &AppSettings,
        cipher: &dyn SecretCipher,
    ) -> Self {
        // Hook 地址不再「只显示一次」：只要密文能解开就回显完整地址（含 token）。
        // 解不开（旧数据 / 主密钥不匹配）时退回遮蔽形式，由前端提示轮换。
        let token = h
            .token_enc
            .as_ref()
            .and_then(|enc| cipher.decrypt(enc).ok());
        let (token_prefix, hook_path, hook_url, hook_url_available) = match token {
            Some(token) => (
                h.token_prefix.clone(),
                alkh_domain::hook::hook_path(&token),
                settings.hook_url(&token),
                true,
            ),
            None => {
                let masked = format!("{}…", h.token_prefix);
                (
                    h.token_prefix.clone(),
                    format!("/api/v1/hooks/alertmanager/{masked}"),
                    settings.hook_url(&masked),
                    false,
                )
            }
        };
        Self {
            id: h.id.clone(),
            name: h.name.clone(),
            token_prefix,
            hook_path,
            hook_url,
            hook_url_available,
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

/// 测试发送的输入（目标渠道由 `send_*` 的 `id` 参数单独给出）。
///
/// 打包成结构体是为了让「落库」与「不落库」两条路径共用同一段实现，
/// 同时避免私有实现的参数表无限增长。
pub struct TestSendInput<'a> {
    pub card_template_id: Option<alkh_domain::ids::CardTemplateId>,
    /// 未保存的草稿模板；`Some` 时忽略策略里的模板引用。
    pub draft: Option<&'a alkh_domain::template::CardTemplate>,
    pub sample: TestSample,
    pub mock: &'a crate::mock::MockSpec,
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
            .map(|h| HookView::from_hook(h, &settings, self.deps.cipher.as_ref()))
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

        let (mut hook, token) = alkh_domain::hook::Hook::issue(channel.id.clone(), "default");
        // Hook 地址要能随时回显：把 token 加密落库（鉴权仍走 token_hash）。
        hook.set_encrypted_token(self.deps.cipher.encrypt(&token)?);
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
                    let (mut hook, token) = alkh_domain::hook::Hook::issue(c.id.clone(), "default");
                    hook.set_encrypted_token(self.deps.cipher.encrypt(&token)?);
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

    /// 向该渠道发送一条测试消息（**落库**：写入「测试分组」与投递记录，可重试 / 可撤回）。
    ///
    /// 用于渠道页的连通性测试：它走完整投递链路，投递记录里能看到 `kind=test`。
    pub async fn send_test(
        &self,
        id: &ChannelId,
        card_template_id: Option<alkh_domain::ids::CardTemplateId>,
        sample: TestSample,
        mock: &crate::mock::MockSpec,
        actor: &AuditActor,
    ) -> AppResult<alkh_domain::alert::Delivery> {
        self.send_test_inner(
            id,
            TestSendInput {
                card_template_id,
                draft: None,
                sample,
                mock,
            },
            actor,
            true,
        )
        .await
    }

    /// 卡片编辑器的「发送测试」：**只把卡片发出去，什么都不落库**。
    ///
    /// 用途是验证卡片样式：不写 `alert_groups`、不写 `deliveries`，所以反复发送
    /// 既不会污染告警分组 / 投递记录，也不会撞 `alert_groups(hook_id, group_key)` 的唯一索引。
    /// 返回的是内存回执（state / message_id / 摘要），**没有重试与撤回语义**。
    pub async fn send_card_test(
        &self,
        id: &ChannelId,
        card_template_id: Option<alkh_domain::ids::CardTemplateId>,
        draft: Option<&alkh_domain::template::CardTemplate>,
        sample: TestSample,
        mock: &crate::mock::MockSpec,
        actor: &AuditActor,
    ) -> AppResult<alkh_domain::alert::Delivery> {
        self.send_test_inner(
            id,
            TestSendInput {
                card_template_id,
                draft,
                sample,
                mock,
            },
            actor,
            false,
        )
        .await
    }

    async fn send_test_inner(
        &self,
        id: &ChannelId,
        input: TestSendInput<'_>,
        actor: &AuditActor,
        record: bool,
    ) -> AppResult<alkh_domain::alert::Delivery> {
        let TestSendInput {
            card_template_id,
            draft,
            sample,
            mock,
        } = input;
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
        let mut group = if mock.alert_group_id.is_some() {
            crate::query::sample_alert_group(&channel, hook_id, resolved, self.deps.now())
        } else {
            mock_source.clone()
        };
        if record {
            // 「测试分组」在库里只能有一行：`alert_groups(hook_id, group_key)` 上有唯一索引。
            // 每次测试都拿新 id 直接 INSERT，第二次测试发送就会撞唯一索引并报
            // `UNIQUE constraint failed: alert_groups.hook_id, alert_groups.group_key`。
            // 这里复用已有行的 id（与真实 intake 的 `find → apply_event → upsert` 同语义），
            // Delivery 引用的分组 id 始终稳定，测试消息依旧可更新 / 可撤回。
            if let Some(existing) = self
                .deps
                .groups
                .find(&group.hook_id, &group.group_key)
                .await?
            {
                group.id = existing.id.clone();
                group.first_seen_at = existing.first_seen_at;
                group.event_count = existing.event_count.saturating_add(1);
            }
            // 必须在发送前落库：Delivery 引用该分组，撤回/重试都需要它存在。
            self.deps.groups.upsert(&group).await?;
        }

        let mut ctx =
            crate::render::build_context(&mock_source, &channel, &policy, self.deps.now());
        mock.apply(&mut ctx);

        let sender = NotificationSender::new(self.deps.clone());
        let prepared = sender
            .prepare_with_context(&group, &channel, &policy, draft, Some(&ctx))
            .await?;
        let delivery = if record {
            sender
                .deliver(
                    &prepared,
                    alkh_domain::alert::DeliveryKind::Test,
                    alkh_domain::alert::DeliveryAction::SendNew,
                    None,
                    None,
                )
                .await?
                .current
        } else {
            // 卡片编辑器的样式验证：只发送，不落库
            sender.send_ephemeral_test(&prepared).await?
        };

        self.audit(
            actor,
            audit_action::TEST_MESSAGE,
            channel.id.as_ref(),
            json!({
                "persisted": record,
                "delivery_id": delivery.id.to_string(),
                "lark_message_id": delivery.lark_message_id.clone(),
                "state": format!("{:?}", delivery.state),
                "draft": draft.map(|t| t.name.clone()),
                "mock_variables": mock.variables.len(),
                "mock_alert_group_id": mock.alert_group_id.as_ref().map(|g| g.to_string()),
            }),
        )
        .await;
        Ok(delivery)
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
