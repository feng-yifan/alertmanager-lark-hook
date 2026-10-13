//! 通知投递器：把「投递决策」翻译成飞书 API 调用，并维护 `Delivery` 生命周期。
//!
//! 覆盖 `docs/business-rules.md` 的 R1（更新降级）、R2（撤回失败不阻塞）、
//! R3（内容未变不重复更新）、R4（即时提醒）、R12（模板缺失兜底）、R14（重试）。

use alkh_domain::alert::{Delivery, DeliveryAction, DeliveryKind, DeliveryState};
use alkh_domain::channel::{Channel, ReceiveIdType};
use alkh_domain::error::LarkError;
use alkh_domain::ids::CardTemplateId;
use alkh_domain::policy::{MessageType, NotificationPolicy};
use alkh_domain::ports::AppFeedCardPayload;
use alkh_domain::template::{
    AppFeedCardSpec, CardSource, CardTemplate, RenderedAppFeedCard, TemplateKind,
};

use crate::deps::Deps;
use crate::error::{AppError, AppResult};
use crate::render::{
    build_context, is_firing_group, render_app_feed_card, render_card_message, RenderedCardMessage,
};

/// 已渲染、待投递的内容快照。
#[derive(Debug, Clone)]
pub enum PreparedContent {
    Card(RenderedCardMessage),
    AppFeed(RenderedAppFeedCard),
}

/// 一次投递所需的全部上下文。
#[derive(Debug, Clone)]
pub struct PreparedSend {
    pub channel: Channel,
    pub group: alkh_domain::alert::AlertGroup,
    pub policy: NotificationPolicy,
    /// 实际使用的模板（内置兜底时为 `None`）。
    pub template_id: Option<CardTemplateId>,
    pub template_kind: TemplateKind,
    pub card_source: Option<CardSource>,
    pub content: PreparedContent,
    pub digest: String,
    pub is_firing: bool,
    /// 应用消息流卡片用于定位/更新的业务 ID。
    pub biz_id: String,
}

/// 投递结果。
#[derive(Debug, Clone)]
pub struct DeliveryResult {
    pub current: Delivery,
    /// 被取代/撤回的旧投递（若本轮有）。
    pub previous: Option<Delivery>,
    /// 是否因 R1 从「原地更新」降级为「新消息」。
    pub fell_back_to_send_new: bool,
}

#[derive(Clone)]
pub struct NotificationSender {
    deps: Deps,
}

impl NotificationSender {
    pub fn new(deps: Deps) -> Self {
        Self { deps }
    }

    /// 渲染并准备一次投递（含模板兜底，R12）。
    pub async fn prepare(
        &self,
        group: &alkh_domain::alert::AlertGroup,
        channel: &Channel,
        policy: &NotificationPolicy,
    ) -> AppResult<PreparedSend> {
        self.prepare_with_context(group, channel, policy, None, None)
            .await
    }

    /// 渲染并准备一次投递，支持显式模板与显式渲染上下文。
    ///
    /// - `explicit`：管理端的**未保存草稿**模板。`None` 时按策略解析（R12 兜底）。
    ///   草稿不落库，因此不会写进 `Delivery.card_template_id`，避免悬空引用。
    /// - `context`：管理端 mock 数据渲染出的上下文。`None` 时用 `group` 现场构造。
    ///   注意 `Delivery` 归属仍由 `group` 决定。
    pub async fn prepare_with_context(
        &self,
        group: &alkh_domain::alert::AlertGroup,
        channel: &Channel,
        policy: &NotificationPolicy,
        explicit: Option<&CardTemplate>,
        context: Option<&alkh_domain::template::RenderContext>,
    ) -> AppResult<PreparedSend> {
        let resolved_template;
        let template: Option<&CardTemplate> = match explicit {
            Some(t) => Some(t),
            None => {
                resolved_template = self
                    .resolve_template(policy, group.channel_id.clone())
                    .await?;
                resolved_template.as_ref()
            }
        };
        let record_template_id = explicit.is_none();

        let owned_context;
        let ctx = match context {
            Some(c) => c,
            None => {
                owned_context = build_context(group, channel, policy, self.deps.now());
                &owned_context
            }
        };
        let is_firing = is_firing_group(group);

        let (template_id, template_kind, card_source, content, digest) = match template {
            Some(t) if t.kind == TemplateKind::CardMessage => {
                let spec = t
                    .card_message
                    .as_ref()
                    .ok_or_else(|| AppError::internal("卡片消息模板缺少 card_message"))?;
                let rendered = render_card_message(spec, ctx).inspect_err(|_| {
                    self.deps
                        .metrics
                        .incr(alkh_domain::ports::Counter::RenderFailure, 1);
                })?;
                let digest = rendered.digest();
                (
                    record_template_id.then(|| t.id.clone()),
                    TemplateKind::CardMessage,
                    Some(spec.source),
                    PreparedContent::Card(rendered),
                    digest,
                )
            }
            Some(t) if t.kind == TemplateKind::AppFeedCard => {
                let spec = t
                    .app_feed_spec
                    .as_ref()
                    .ok_or_else(|| AppError::internal("应用消息流模板缺少 app_feed_spec"))?;
                let rendered = render_app_feed_card(spec, ctx, is_firing);
                let digest = digest_of_app_feed(&rendered);
                (
                    record_template_id.then(|| t.id.clone()),
                    TemplateKind::AppFeedCard,
                    None,
                    PreparedContent::AppFeed(rendered),
                    digest,
                )
            }
            _ => {
                // 内置兜底，保证告警不丢（R12）。
                match policy.message_type {
                    MessageType::CardMessage => {
                        let spec = alkh_domain::template::CardMessageSpec::builder_json(
                            crate::defaults::default_card_json(),
                        );
                        let rendered = render_card_message(&spec, ctx).inspect_err(|_| {
                            self.deps
                                .metrics
                                .incr(alkh_domain::ports::Counter::RenderFailure, 1);
                        })?;
                        let digest = rendered.digest();
                        (
                            None,
                            TemplateKind::CardMessage,
                            Some(CardSource::BuilderJson),
                            PreparedContent::Card(rendered),
                            digest,
                        )
                    }
                    MessageType::AppFeedCard => {
                        let spec = AppFeedCardSpec::default();
                        let rendered = render_app_feed_card(&spec, ctx, is_firing);
                        let digest = digest_of_app_feed(&rendered);
                        (
                            None,
                            TemplateKind::AppFeedCard,
                            None,
                            PreparedContent::AppFeed(rendered),
                            digest,
                        )
                    }
                }
            }
        };

        let biz_id = match template {
            Some(t) => match t.app_feed_spec.as_ref().map(|s| s.biz_id_source) {
                Some(alkh_domain::template::BizIdSource::Hook) => {
                    format!("alkh-{}-{}", group.hook_id, group.id)
                }
                _ => format!("alkh-{}", group.id),
            },
            None => format!("alkh-{}", group.id),
        };

        Ok(PreparedSend {
            channel: channel.clone(),
            group: group.clone(),
            policy: policy.clone(),
            template_id,
            template_kind,
            card_source,
            content,
            digest,
            is_firing,
            biz_id,
        })
    }

    /// 解析策略引用的模板；返回 `None` 表示需要用内置兜底。
    async fn resolve_template(
        &self,
        policy: &NotificationPolicy,
        channel_id: alkh_domain::ids::ChannelId,
    ) -> AppResult<Option<CardTemplate>> {
        let want = match policy.message_type {
            MessageType::CardMessage => TemplateKind::CardMessage,
            MessageType::AppFeedCard => TemplateKind::AppFeedCard,
        };

        if let Some(id) = &policy.card_template_id {
            if let Some(t) = self.deps.templates.get(id).await? {
                if t.kind == want {
                    return Ok(Some(t));
                }
                tracing::warn!(
                    template_id = %id,
                    ?want,
                    "策略引用的模板类型不匹配，回退到默认模板"
                );
            } else {
                tracing::warn!(template_id = %id, "策略引用的模板不存在，回退到默认模板");
            }
        }
        let _ = channel_id;
        Ok(self.deps.templates.find_default(want).await?)
    }

    /// 执行投递决策。
    pub async fn deliver(
        &self,
        prepared: &PreparedSend,
        kind: DeliveryKind,
        action: DeliveryAction,
        previous: Option<&Delivery>,
        idempotency_key: Option<&str>,
    ) -> AppResult<DeliveryResult> {
        match action {
            DeliveryAction::Suppress => Err(AppError::internal(
                "Suppress 动作不应进入投递器（调用方需先处理）",
            )),
            DeliveryAction::SendNew => {
                let current = self.send_new(prepared, kind, idempotency_key).await?;
                let previous = self.supersede(previous).await;
                Ok(DeliveryResult {
                    current,
                    previous,
                    fell_back_to_send_new: false,
                })
            }
            DeliveryAction::UpdateInPlace => match previous {
                Some(prev) => {
                    self.update_in_place(prepared, prev, kind, idempotency_key)
                        .await
                }
                None => {
                    let current = self.send_new(prepared, kind, idempotency_key).await?;
                    Ok(DeliveryResult {
                        current,
                        previous: None,
                        fell_back_to_send_new: true,
                    })
                }
            },
            DeliveryAction::SendNewThenRevoke => {
                let current = self.send_new(prepared, kind, idempotency_key).await?;
                // ⚠️ 只有新消息**真的发出去了**才撤回旧消息。
                //
                // `send_new` 在发送失败时同样返回 `Ok`（状态为 Pending/Failed，交给后台重试），
                // 所以不能无条件撤回：那样会出现「旧消息已撤回、新消息没发出」，
                // 这条告警在群里彻底消失。失败时保留旧消息，用户至少还能看到它。
                let previous = match (previous, &current.state) {
                    (Some(prev), DeliveryState::Active | DeliveryState::Resolved) => {
                        Some(self.revoke(prepared, prev).await)
                    }
                    (Some(_), state) => {
                        tracing::warn!(
                            delivery_id = %current.id,
                            state = ?state,
                            "新消息未发送成功，保留旧消息不撤回（避免告警消息在群里消失）"
                        );
                        None
                    }
                    (None, _) => None,
                };
                Ok(DeliveryResult {
                    current,
                    previous,
                    fell_back_to_send_new: false,
                })
            }
        }
    }

    /// 发送一条全新消息。
    pub async fn send_new(
        &self,
        prepared: &PreparedSend,
        kind: DeliveryKind,
        idempotency_key: Option<&str>,
    ) -> AppResult<Delivery> {
        let now = self.deps.now();
        let mut delivery = Delivery::new(
            prepared.group.id.clone(),
            prepared.channel.id.clone(),
            kind,
            prepared.policy.message_type,
            prepared.template_id.clone(),
            prepared.digest.clone(),
            now,
        );
        self.deps.deliveries.insert(&delivery).await?;

        let uuid = message_uuid(idempotency_key);
        match self.perform_send(prepared, &mut delivery, &uuid).await {
            Ok(()) => {
                delivery.mark_active(self.deps.now());
            }
            Err(err) => {
                self.register_failure(&mut delivery, err);
            }
        }
        self.deps.deliveries.update(&delivery).await?;
        Ok(delivery)
    }

    /// 重试一条已存在的投递（后台重试任务 / 手动重试）。
    pub async fn retry(&self, delivery: &Delivery) -> AppResult<Delivery> {
        let group = self
            .deps
            .groups
            .get(&delivery.alert_group_id)
            .await?
            .ok_or_else(|| AppError::not_found("告警分组不存在，无法重试"))?;
        let channel = self
            .deps
            .channels
            .get(&delivery.channel_id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在，无法重试"))?;
        let policy = self
            .deps
            .policies
            .get(&delivery.channel_id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(delivery.channel_id.clone()));

        let prepared = self.prepare(&group, &channel, &policy).await?;
        let mut d = delivery.clone();
        if d.state == DeliveryState::Failed {
            // 手动重试：重置尝试计数，重新走完整重试预算。
            d.attempt = 0;
        }
        // 自动重试（后台任务）保留 attempt，避免无限重试。
        d.state = DeliveryState::Pending;
        let uuid = message_uuid(None);
        let now = self.deps.now();
        match self.perform_send(&prepared, &mut d, &uuid).await {
            Ok(()) => {
                d.mark_active(now);
                // 记录**实际发出**的内容，否则投递里留的还是首次失败时的旧 digest，
                // 会让后续同内容事件被误判为「内容已变」而多做一次更新。
                d.rendered_digest = prepared.digest.clone();
                // 重试成功后，这条消息就是该分组当前在群里的活跃消息，必须回写指针。
                // 否则后续 `Updated` 事件会以为「没有旧消息可更新」而连发新卡片
                // （`group.active_delivery_id` 是唯一的活跃消息指针，`find_active` 在生产未使用）。
                if let Err(e) = self
                    .deps
                    .groups
                    .set_active_delivery(&d.alert_group_id, Some(&d.id), Some(now))
                    .await
                {
                    tracing::warn!(
                        delivery_id = %d.id,
                        error = %e,
                        "重试成功但回写分组活跃投递指针失败：下一次更新可能重复发卡片"
                    );
                }
            }
            Err(err) => self.register_failure(&mut d, err),
        }
        self.deps.deliveries.update(&d).await?;
        Ok(d)
    }

    /// 原地更新当前活跃消息（R1/R3）。
    async fn update_in_place(
        &self,
        prepared: &PreparedSend,
        previous: &Delivery,
        kind: DeliveryKind,
        idempotency_key: Option<&str>,
    ) -> AppResult<DeliveryResult> {
        // R3：内容未变化则跳过（幂等，避免无意义更新与限频）。
        if previous.rendered_digest == prepared.digest && previous.has_message_ref() {
            return Ok(DeliveryResult {
                current: previous.clone(),
                previous: None,
                fell_back_to_send_new: false,
            });
        }

        // R1：没有可更新的消息引用 ⇒ 降级为新消息。
        if !previous.has_message_ref() {
            let current = self.send_new(prepared, kind, idempotency_key).await?;
            return Ok(DeliveryResult {
                current,
                previous: None,
                fell_back_to_send_new: true,
            });
        }

        let now = self.deps.now();
        let mut d = previous.clone();
        d.kind = kind;
        // ⚠️ 这里**只**更新元数据，不碰 `rendered_digest`。
        //
        // 旧实现把新 digest 写在这里（调用飞书之前），失败分支又把它落库，
        // 于是下一次相同内容的事件会命中下面的 R3 提前返回被跳过 ——
        // 卡片永久停在旧内容且无法自愈。digest 只能在更新**成功**后推进。
        d.card_template_id = prepared.template_id.clone();
        d.updated_at = now;

        let uuid = message_uuid(idempotency_key);
        let outcome: AppResult<()> = match (&prepared.content, prepared.policy.message_type) {
            (PreparedContent::Card(RenderedCardMessage::Json(card)), MessageType::CardMessage) => {
                match d.card_entity_id.clone() {
                    Some(card_id) => {
                        let seq = d.next_sequence();
                        self.deps
                            .lark
                            .update_card_entity(&card_id, card, seq, &uuid)
                            .await
                            .map_err(AppError::from)
                    }
                    None => match d.lark_message_id.clone() {
                        Some(msg_id) => self
                            .deps
                            .lark
                            .update_message_content(&msg_id, card)
                            .await
                            .map_err(AppError::from),
                        None => Err(AppError::internal("缺少卡片引用")),
                    },
                }
            }
            (PreparedContent::Card(rendered), MessageType::CardMessage) => {
                let msg_id = d.lark_message_id.clone().unwrap_or_default();
                self.deps
                    .lark
                    .update_message_content(&msg_id, &rendered.to_content())
                    .await
                    .map_err(AppError::from)
            }
            (PreparedContent::AppFeed(rendered), MessageType::AppFeedCard) => {
                let biz_id = d
                    .app_feed_biz_id
                    .clone()
                    .unwrap_or_else(|| prepared.biz_id.clone());
                let payload = AppFeedCardPayload::from_rendered(
                    rendered.clone(),
                    Some(biz_id),
                    self.time_sensitive(prepared),
                );
                self.deps
                    .lark
                    .update_app_feed_card(
                        &prepared.channel.receive_id,
                        prepared.channel.receive_id_type,
                        &payload,
                    )
                    .await
                    .map_err(AppError::from)
            }
            _ => Err(AppError::internal("消息类型与内容不匹配")),
        };

        match outcome {
            Ok(()) => {
                // 更新确实生效了，此时才推进 digest / 模板引用。
                d.rendered_digest = prepared.digest.clone();
                if kind == DeliveryKind::Resolved {
                    d.mark_resolved(self.deps.now());
                } else {
                    d.mark_active(self.deps.now());
                }
                self.deps.deliveries.update(&d).await?;
                Ok(DeliveryResult {
                    current: d,
                    previous: None,
                    fell_back_to_send_new: false,
                })
            }
            Err(AppError::Lark(err)) if is_update_unavailable(&err) => {
                // R1：实体过期 / 消息已删除 / 应用不匹配 ⇒ 降级为新消息。
                tracing::warn!(
                    delivery_id = %d.id,
                    code = err.code,
                    "原地更新不可用，降级为新消息"
                );
                let current = self.send_new(prepared, kind, idempotency_key).await?;
                Ok(DeliveryResult {
                    current,
                    previous: None,
                    fell_back_to_send_new: true,
                })
            }
            Err(err) => {
                self.register_failure(&mut d, err);
                self.deps.deliveries.update(&d).await?;
                Ok(DeliveryResult {
                    current: d,
                    previous: None,
                    fell_back_to_send_new: false,
                })
            }
        }
    }

    /// 把上一条投递标记为「被取代」。
    async fn supersede(&self, previous: Option<&Delivery>) -> Option<Delivery> {
        let prev = previous?;
        if prev.state != DeliveryState::Active {
            return None;
        }
        let mut d = prev.clone();
        d.mark_superseded(self.deps.now());
        if self.deps.deliveries.update(&d).await.is_err() {
            return None;
        }
        Some(d)
    }

    /// 撤回 / 删除上一条消息（R2：失败不阻塞主流程）。
    pub async fn revoke(&self, prepared: &PreparedSend, previous: &Delivery) -> Delivery {
        let now = self.deps.now();
        let mut d = previous.clone();
        let outcome = match prepared.policy.message_type {
            MessageType::CardMessage => match d.lark_message_id.clone() {
                Some(id) => self.deps.lark.recall_message(&id).await,
                None => return d,
            },
            MessageType::AppFeedCard => match d.app_feed_biz_id.clone() {
                Some(biz) => {
                    self.deps
                        .lark
                        .delete_app_feed_card(
                            &prepared.channel.receive_id,
                            prepared.channel.receive_id_type,
                            &biz,
                        )
                        .await
                }
                None => return d,
            },
        };
        match outcome {
            Ok(()) => {
                d.mark_recalled(now);
                self.deps
                    .metrics
                    .incr(alkh_domain::ports::Counter::DeliveryRecalled, 1);
            }
            Err(err) => {
                tracing::warn!(delivery_id = %d.id, error = %err, "撤回旧消息失败（R2：不阻塞主流程）");
                self.deps
                    .metrics
                    .incr(alkh_domain::ports::Counter::LarkApiError, 1);
                d.mark_recall_failed(err.to_string(), now);
            }
        }
        if let Err(e) = self.deps.deliveries.update(&d).await {
            tracing::error!(delivery_id = %d.id, error = %e, "撤回状态落库失败");
        }
        d
    }

    /// 手动撤回（管理后台触发）。
    pub async fn revoke_delivery(&self, delivery: &Delivery) -> AppResult<Delivery> {
        let channel = self
            .deps
            .channels
            .get(&delivery.channel_id)
            .await?
            .ok_or_else(|| AppError::not_found("渠道不存在"))?;
        let policy = self
            .deps
            .policies
            .get(&delivery.channel_id)
            .await?
            .unwrap_or_else(|| NotificationPolicy::default_for(delivery.channel_id.clone()));
        let group = self
            .deps
            .groups
            .get(&delivery.alert_group_id)
            .await?
            .ok_or_else(|| AppError::not_found("告警分组不存在"))?;
        // 撤回前先记下分组当前指向谁：只有它指向的就是被撤回的这条时，
        // 才应该把指针清掉（否则会误伤更新的活跃投递）。
        let pointer_is_this = group.active_delivery_id.as_ref() == Some(&delivery.id);
        let prepared = PreparedSend {
            channel,
            group,
            policy,
            template_id: delivery.card_template_id.clone(),
            template_kind: TemplateKind::CardMessage,
            card_source: None,
            content: PreparedContent::Card(RenderedCardMessage::Json(serde_json::json!({}))),
            digest: delivery.rendered_digest.clone(),
            is_firing: false,
            biz_id: delivery.app_feed_biz_id.clone().unwrap_or_default(),
        };
        let d = self.revoke(&prepared, delivery).await;
        if d.state == DeliveryState::RecallFailed {
            return Err(AppError::Lark(LarkError::new(
                -1,
                d.last_error.clone().unwrap_or_else(|| "撤回失败".into()),
            )));
        }
        // 消息已经被撤掉了，分组不应再指向它；否则下一个事件会拿一条已撤回的投递
        // 去尝试原地更新，失败后再降级发新消息，表现为重复卡片。
        if pointer_is_this {
            if let Err(e) = self
                .deps
                .groups
                .set_active_delivery(&d.alert_group_id, None, None)
                .await
            {
                tracing::warn!(
                    delivery_id = %d.id,
                    error = %e,
                    "撤回成功但清空分组活跃投递指针失败"
                );
            }
        }
        Ok(d)
    }

    /// R4：`pin_unresolved` 仅对应用消息流卡片、且消息含 firing 告警时生效。
    fn time_sensitive(&self, prepared: &PreparedSend) -> bool {
        prepared.policy.pin_unresolved
            && prepared.is_firing
            && prepared.policy.message_type == MessageType::AppFeedCard
    }

    async fn perform_send(
        &self,
        prepared: &PreparedSend,
        delivery: &mut Delivery,
        uuid: &str,
    ) -> AppResult<()> {
        match (&prepared.content, prepared.policy.message_type) {
            (PreparedContent::Card(RenderedCardMessage::Json(card)), MessageType::CardMessage) => {
                let card_id = self.deps.lark.create_card_entity(card).await?;
                let outcome = self
                    .deps
                    .lark
                    .send_card_entity(
                        &prepared.channel.receive_id,
                        prepared.channel.receive_id_type,
                        &card_id,
                        uuid,
                    )
                    .await?;
                delivery.card_entity_id = Some(card_id);
                delivery.lark_message_id = outcome.message_id;
            }
            (PreparedContent::Card(rendered), MessageType::CardMessage) => {
                let outcome = self
                    .deps
                    .lark
                    .send_message_content(
                        &prepared.channel.receive_id,
                        prepared.channel.receive_id_type,
                        &rendered.to_content(),
                        uuid,
                    )
                    .await?;
                delivery.lark_message_id = outcome.message_id;
            }
            (PreparedContent::AppFeed(rendered), MessageType::AppFeedCard) => {
                let payload = AppFeedCardPayload::from_rendered(
                    rendered.clone(),
                    Some(prepared.biz_id.clone()),
                    self.time_sensitive(prepared),
                );
                let outcome = self
                    .deps
                    .lark
                    .send_app_feed_card(
                        &prepared.channel.receive_id,
                        prepared.channel.receive_id_type,
                        &payload,
                    )
                    .await?;
                if !outcome.failed_cards.is_empty() {
                    return Err(AppError::Lark(alkh_domain::ports::feed_card_failure(
                        "发送应用消息流卡片部分失败",
                        &outcome.failed_cards,
                    )));
                }
                delivery.app_feed_biz_id = Some(
                    outcome
                        .app_feed_biz_id
                        .unwrap_or_else(|| prepared.biz_id.clone()),
                );
            }
            _ => return Err(AppError::internal("消息类型与模板类型不匹配")),
        }

        if !delivery.has_message_ref() {
            return Err(AppError::internal(
                "飞书未返回消息引用（message_id / biz_id）",
            ));
        }
        Ok(())
    }

    /// 记录一次失败；可重试且未耗尽时保持 `Pending` 交给后台任务。
    fn register_failure(&self, delivery: &mut Delivery, err: AppError) {
        let now = self.deps.now();
        // 飞书侧报错单独计数：这是运维判断「是不是飞书在抖」的第一手信号。
        if matches!(&err, AppError::Lark(_)) {
            self.deps
                .metrics
                .incr(alkh_domain::ports::Counter::LarkApiError, 1);
        }
        // 配置类问题（如未填写飞书凭据）在修复后应能自动重试，因此保持 Pending。
        let retryable = matches!(&err, AppError::Lark(l) if l.retryable)
            || matches!(&err, AppError::Internal(_))
            || matches!(&err, AppError::LarkNotConfigured(_));
        let msg = err.to_string();
        delivery.record_attempt(Some(msg.clone()), now);
        if !retryable || delivery.exhausted() {
            delivery.mark_failed(msg, now);
        }
        // 否则保持 Pending，等待重试任务。
    }
}

fn digest_of_app_feed(rendered: &RenderedAppFeedCard) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(rendered).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hex::encode(&hasher.finalize()[..16])
}

/// 飞书 `uuid` 去重键，最长 50 字符（R9）。
pub fn message_uuid(idempotency_key: Option<&str>) -> String {
    match idempotency_key {
        Some(k) if !k.is_empty() => {
            let trimmed: String = k.chars().take(45).collect();
            format!("alkh:{trimmed}")
        }
        _ => format!("alkh:{}", uuid::Uuid::new_v4()),
    }
}

/// 判断飞书错误是否意味着「这条消息/卡片已经不能被原地更新」（R1）。
pub fn is_update_unavailable(err: &LarkError) -> bool {
    matches!(
        err.code,
        // 卡片实体不存在 / 已过期 / 应用不匹配
        200740 | 200750 | 200860 | 300311
        // 消息已删除 / 已过期 / 不可见
        | 230110 | 230009 | 230050
            // 应用消息流卡片不存在（本适配器由 failed_cards 的 reason 归一而来）
            | LarkError::CODE_FEED_CARD_NOT_FOUND
    )
}

/// 该渠道是否满足应用消息流卡片的投递前提（R15）。
pub fn app_feed_supported(receive_id_type: ReceiveIdType) -> bool {
    matches!(
        receive_id_type,
        ReceiveIdType::OpenId | ReceiveIdType::UserId
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_unavailable_codes() {
        assert!(is_update_unavailable(&LarkError::new(200750, "expired")));
        assert!(is_update_unavailable(&LarkError::new(230110, "deleted")));
        assert!(!is_update_unavailable(&LarkError::new(
            230020,
            "rate limited"
        )));
        assert!(!is_update_unavailable(&LarkError::new(0, "ok")));
        // 应用消息流卡片不存在（由 failed_cards 的 reason 归一而来）也要触发 R1 降级，
        // 否则 app feed 的原地更新失效后不会改发新消息。
        assert!(is_update_unavailable(&LarkError::new(
            LarkError::CODE_FEED_CARD_NOT_FOUND,
            "feed card not created"
        )));
    }

    #[test]
    fn uuid_is_bounded_and_deterministic_when_keyed() {
        let key = "a".repeat(80);
        let u = message_uuid(Some(&key));
        assert!(u.len() <= 50, "uuid 过长: {}", u.len());
        assert_eq!(u, message_uuid(Some(&key)));
        let random = message_uuid(None);
        assert_ne!(random, message_uuid(None));
        assert!(random.starts_with("alkh:"));
    }

    #[test]
    fn app_feed_only_for_user_ids() {
        assert!(app_feed_supported(ReceiveIdType::OpenId));
        assert!(app_feed_supported(ReceiveIdType::UserId));
        assert!(!app_feed_supported(ReceiveIdType::ChatId));
        assert!(!app_feed_supported(ReceiveIdType::Email));
    }
}
