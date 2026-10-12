//! 飞书开放平台网关。
//!
//! 端点与字段名严格对应 `docs/research/feishu-api.md`：
//!
//! | 用途 | 方法 + 路径 |
//! |---|---|
//! | tenant_access_token | `POST /open-apis/auth/v3/tenant_access_token/internal` |
//! | 群列表 | `GET /open-apis/im/v1/chats` |
//! | 创建卡片实体 | `POST /open-apis/cardkit/v1/cards` |
//! | 全量更新卡片实体 | `PUT /open-apis/cardkit/v1/cards/:card_id` |
//! | 发送消息 | `POST /open-apis/im/v1/messages?receive_id_type=` |
//! | 更新已发卡片消息 | `PATCH /open-apis/im/v1/messages/:message_id` |
//! | 撤回消息 | `DELETE /open-apis/im/v1/messages/:message_id` |
//! | 发送应用消息流卡片 | `POST /open-apis/im/v2/app_feed_card` |
//! | 更新应用消息流卡片 | `PUT /open-apis/im/v2/app_feed_card/batch` |
//! | 删除应用消息流卡片 | `DELETE /open-apis/im/v2/app_feed_card/batch` |

use std::sync::Arc;
use std::time::{Duration, Instant};

use alkh_domain::channel::ReceiveIdType;
use alkh_domain::error::{LarkError, PortError, PortResult};
use alkh_domain::ports::{
    AppFeedCardPayload, LarkChat, LarkCredential, LarkCredentials, LarkFailedCard, LarkGateway,
    LarkSendOutcome,
};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;

/// 提前 5 分钟刷新 token（飞书 token 有效期约 7200s）。
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;
/// 单个 HTTP 请求的最大重试次数（限频 / 5xx / 网络错误）。
const MAX_ATTEMPTS: u32 = 3;
/// 应用消息流卡片更新字段：标题 / 预览 / 状态标签 / 按钮 / 跳转链接 / 即时提醒 / 进行通知。
const FEED_UPDATE_FIELDS: [&str; 7] = ["1", "3", "10", "11", "12", "13", "103"];

struct CachedToken {
    token: String,
    expires_at: Instant,
    fingerprint: String,
}

/// 飞书网关实现。
pub struct LarkClient {
    http: reqwest::Client,
    credentials: Arc<dyn LarkCredentials>,
    token: RwLock<Option<CachedToken>>,
}

impl LarkClient {
    pub fn new(credentials: Arc<dyn LarkCredentials>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!(
                "alertmanager-lark-hook/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;
        Ok(Self {
            http,
            credentials,
            token: RwLock::new(None),
        })
    }

    /// 使缓存的 token 失效（凭据变更或 token 报错时调用）。
    pub async fn invalidate_token(&self) {
        *self.token.write().await = None;
    }

    async fn credential(&self) -> PortResult<LarkCredential> {
        self.credentials.current().await?.ok_or_else(|| {
            PortError::NotConfigured("尚未配置飞书应用凭据（app_id / app_secret）".into())
        })
    }

    async fn access_token(&self) -> PortResult<(String, LarkCredential)> {
        let cred = self.credential().await?;
        let fingerprint = credential_fingerprint(&cred);

        {
            let guard = self.token.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fingerprint == fingerprint && cached.expires_at > Instant::now() {
                    return Ok((cached.token.clone(), cred));
                }
            }
        }

        let url = format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            base_url(&cred)
        );
        let body = json!({ "app_id": cred.app_id, "app_secret": cred.app_secret });
        let (token, expire) = self.fetch_token(&url, &body).await?;

        let ttl = expire.saturating_sub(TOKEN_REFRESH_MARGIN_SECS).max(60);
        *self.token.write().await = Some(CachedToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(ttl),
            fingerprint,
        });
        Ok((token, cred))
    }

    async fn fetch_token(&self, url: &str, body: &Value) -> PortResult<(String, u64)> {
        let resp = self
            .http
            .post(url)
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await
            .map_err(|e| PortError::Unavailable(format!("请求飞书 token 接口失败: {e}")))?;

        let status = resp.status().as_u16();
        let log_id = header(resp.headers(), "x-tt-logid");
        let text = resp.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

        let code = parsed.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if status < 300 && code == 0 {
            let token = parsed
                .get("tenant_access_token")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    PortError::Lark(LarkError::new(-1, "响应缺少 tenant_access_token"))
                })?;
            let expire = parsed.get("expire").and_then(Value::as_u64).unwrap_or(7200);
            return Ok((token.to_string(), expire));
        }

        Err(PortError::Lark(LarkError {
            code,
            message: format!(
                "{} (http {status})",
                parsed
                    .get("msg")
                    .and_then(Value::as_str)
                    .unwrap_or("获取 tenant_access_token 失败")
            ),
            log_id,
            http_status: Some(status),
            retryable: status == 429 || status >= 500,
        }))
    }

    /// 统一请求：鉴权头、错误码解析、限频/5xx/网络退避重试、token 失效重试。
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> PortResult<Value> {
        let mut attempt = 0u32;
        let mut refreshed = false;

        loop {
            attempt += 1;
            let (token, cred) = self.access_token().await?;
            let url = format!("{}{}", base_url(&cred), path);

            let mut req = self
                .http
                .request(method.clone(), &url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json; charset=utf-8");
            if let Some(b) = body {
                req = req.json(b);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    let transient = e.is_timeout() || e.is_connect() || e.is_request();
                    if transient && attempt < MAX_ATTEMPTS {
                        sleep_backoff(attempt).await;
                        continue;
                    }
                    return Err(PortError::Unavailable(format!("请求飞书失败: {e}")));
                }
            };

            let status = resp.status().as_u16();
            let log_id = header(resp.headers(), "x-tt-logid");
            let retry_after =
                header(resp.headers(), "retry-after").and_then(|v| v.parse::<u64>().ok());
            let text = resp.text().await.unwrap_or_default();
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

            let code = parsed
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or(if status < 300 { 0 } else { -1 });
            let msg = parsed
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if status < 300 && code == 0 {
                return Ok(parsed.get("data").cloned().unwrap_or(Value::Null));
            }

            // token 失效：清缓存后重试一次
            if !refreshed && (status == 401 || is_token_error(code)) {
                refreshed = true;
                self.invalidate_token().await;
                continue;
            }

            let retryable = status == 429 || status >= 500 || code == LarkError::CODE_RATE_LIMITED;
            if retryable && attempt < MAX_ATTEMPTS {
                let wait = retry_after
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| backoff(attempt));
                tracing::warn!(
                    path,
                    code,
                    status,
                    wait_ms = wait.as_millis() as u64,
                    "飞书限频/服务端错误，退避重试"
                );
                tokio::time::sleep(wait).await;
                continue;
            }

            return Err(PortError::Lark(LarkError {
                code,
                message: if msg.is_empty() {
                    format!("HTTP {status}")
                } else {
                    format!("{msg} (http {status})")
                },
                log_id,
                http_status: Some(status),
                retryable,
            }));
        }
    }
}

fn base_url(cred: &LarkCredential) -> String {
    cred.base_url.trim_end_matches('/').to_string()
}

/// 凭据指纹：app_id、基址或 app_secret 任一变化都会导致 token 缓存失效。
fn credential_fingerprint(cred: &LarkCredential) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(cred.app_id.as_bytes());
    hasher.update(b"|");
    hasher.update(cred.base_url.as_bytes());
    hasher.update(b"|");
    hasher.update(cred.app_secret.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

fn header(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(300 * 2u64.pow(attempt.min(5)))
}

async fn sleep_backoff(attempt: u32) {
    tokio::time::sleep(backoff(attempt)).await;
}

/// 飞书在 token 失效时返回的错误码。
fn is_token_error(code: i64) -> bool {
    matches!(code, 99991663 | 99991664 | 99991661 | 99991668 | 99991677)
}

fn user_id_type_str(t: ReceiveIdType) -> PortResult<&'static str> {
    match t {
        ReceiveIdType::OpenId => Ok("open_id"),
        ReceiveIdType::UserId => Ok("user_id"),
        ReceiveIdType::ChatId => Err(PortError::Internal(
            "应用消息流卡片不支持 chat_id（该接口只有用户维度）".into(),
        )),
        ReceiveIdType::Email => Err(PortError::Internal(
            "应用消息流卡片不支持 email，请使用 open_id / user_id".into(),
        )),
    }
}

/// 构造 `open_app_feed_card` JSON。
fn app_feed_card_json(payload: &AppFeedCardPayload, include_biz_id: bool) -> Value {
    let mut card = Map::new();
    if include_biz_id {
        if let Some(biz) = &payload.biz_id {
            card.insert("biz_id".into(), json!(biz));
        }
    } else if let Some(biz) = &payload.biz_id {
        card.insert("biz_id".into(), json!(biz));
    }
    card.insert("title".into(), json!(payload.title));
    card.insert("preview".into(), json!(payload.preview));

    if let Some(label) = &payload.status_label {
        card.insert(
            "status_label".into(),
            json!({ "text": label.text, "type": label.label_type.as_str() }),
        );
    }
    if let Some(link) = &payload.link {
        card.insert("link".into(), json!({ "link": link }));
    }
    if !payload.buttons.is_empty() {
        let buttons: Vec<Value> = payload
            .buttons
            .iter()
            .map(|b| {
                json!({
                    "action_type": "url_page",
                    "text": { "text": b.text },
                    "button_type": b.button_type.as_str(),
                    "multi_url": { "url": b.url }
                })
            })
            .collect();
        card.insert("buttons".into(), json!({ "buttons": buttons }));
    }
    card.insert("time_sensitive".into(), json!(payload.time_sensitive));
    card.insert(
        "notify".into(),
        json!({ "close_notify": payload.close_notify }),
    );
    Value::Object(card)
}

/// 数据包装成 JSON 字符串（飞书要求 `content` / `data` 为转义后的字符串）。
fn as_json_string(v: &Value) -> PortResult<String> {
    serde_json::to_string(v).map_err(|e| PortError::Internal(format!("JSON 序列化失败: {e}")))
}

fn parse_failed_cards(data: &Value) -> Vec<LarkFailedCard> {
    data.get("failed_cards")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|c| LarkFailedCard {
                    biz_id: c.get("biz_id").and_then(Value::as_str).map(str::to_string),
                    user_id: c.get("user_id").and_then(Value::as_str).map(str::to_string),
                    reason: c.get("reason").map(|r| {
                        r.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| r.to_string())
                    }),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl LarkGateway for LarkClient {
    async fn verify_credentials(&self) -> PortResult<Vec<LarkChat>> {
        self.list_chats().await
    }

    async fn list_chats(&self) -> PortResult<Vec<LarkChat>> {
        let mut chats = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut path = String::from("/open-apis/im/v1/chats?page_size=100");
            if let Some(pt) = &page_token {
                path.push_str("&page_token=");
                path.push_str(&urlencoding::encode(pt));
            }
            let data = self.call(reqwest::Method::GET, &path, None).await?;

            if let Some(items) = data.get("items").and_then(Value::as_array) {
                for item in items {
                    let chat_status = item
                        .get("chat_status")
                        .and_then(Value::as_str)
                        .unwrap_or("normal");
                    chats.push(LarkChat {
                        chat_id: item
                            .get("chat_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        avatar_url: item
                            .get("avatar")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        description: item
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        external: item
                            .get("external")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        dissolved: matches!(chat_status, "dissolved" | "dissolved_save"),
                    });
                }
            }

            let has_more = data
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            page_token = data
                .get("page_token")
                .and_then(Value::as_str)
                .map(str::to_string);
            if !has_more || page_token.is_none() {
                break;
            }
        }
        Ok(chats)
    }

    async fn create_card_entity(&self, card_json: &Value) -> PortResult<String> {
        let body = json!({
            "type": "card_json",
            "data": as_json_string(card_json)?
        });
        let data = self
            .call(
                reqwest::Method::POST,
                "/open-apis/cardkit/v1/cards",
                Some(&body),
            )
            .await?;
        data.get("card_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| PortError::Lark(LarkError::new(-1, "创建卡片实体未返回 card_id")))
    }

    async fn create_card_entity_from_template(
        &self,
        template_id: &str,
        template_version_name: Option<&str>,
        variables: &Map<String, Value>,
    ) -> PortResult<String> {
        let mut inner = Map::new();
        inner.insert("template_id".into(), json!(template_id));
        if let Some(v) = template_version_name.filter(|v| !v.trim().is_empty()) {
            inner.insert("template_version_name".into(), json!(v));
        }
        inner.insert("template_variable".into(), Value::Object(variables.clone()));

        let body = json!({
            "type": "template",
            "data": as_json_string(&Value::Object(inner))?
        });
        let data = self
            .call(
                reqwest::Method::POST,
                "/open-apis/cardkit/v1/cards",
                Some(&body),
            )
            .await?;
        data.get("card_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| PortError::Lark(LarkError::new(-1, "创建模板卡片实体未返回 card_id")))
    }

    async fn send_card_entity(
        &self,
        receive_id: &str,
        receive_id_type: ReceiveIdType,
        card_entity_id: &str,
        uuid: &str,
    ) -> PortResult<LarkSendOutcome> {
        // 文档明确：msg_type=interactive 时 content = {"type":"card","data":{"card_id":"…"}}
        let content = json!({ "type": "card", "data": { "card_id": card_entity_id } });
        self.send_message_content(receive_id, receive_id_type, &content, uuid)
            .await
    }

    async fn send_message_content(
        &self,
        receive_id: &str,
        receive_id_type: ReceiveIdType,
        content: &Value,
        uuid: &str,
    ) -> PortResult<LarkSendOutcome> {
        let path = format!(
            "/open-apis/im/v1/messages?receive_id_type={}",
            receive_id_type.as_str()
        );
        let body = json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": as_json_string(content)?,
            "uuid": uuid,
        });
        let data = self.call(reqwest::Method::POST, &path, Some(&body)).await?;
        Ok(LarkSendOutcome {
            message_id: data
                .get("message_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            app_feed_biz_id: None,
            failed_cards: Vec::new(),
        })
    }

    async fn update_card_entity(
        &self,
        card_entity_id: &str,
        card_json: &Value,
        sequence: i64,
        uuid: &str,
    ) -> PortResult<()> {
        let path = format!("/open-apis/cardkit/v1/cards/{card_entity_id}");
        let body = json!({
            "card": { "type": "card_json", "data": as_json_string(card_json)? },
            "uuid": uuid,
            "sequence": sequence,
        });
        self.call(reqwest::Method::PUT, &path, Some(&body)).await?;
        Ok(())
    }

    async fn update_message_content(&self, message_id: &str, content: &Value) -> PortResult<()> {
        let path = format!("/open-apis/im/v1/messages/{message_id}");
        let body = json!({ "content": as_json_string(content)? });
        self.call(reqwest::Method::PATCH, &path, Some(&body))
            .await?;
        Ok(())
    }

    async fn recall_message(&self, message_id: &str) -> PortResult<()> {
        let path = format!("/open-apis/im/v1/messages/{message_id}");
        self.call(reqwest::Method::DELETE, &path, None).await?;
        Ok(())
    }

    async fn send_app_feed_card(
        &self,
        user_id: &str,
        user_id_type: ReceiveIdType,
        payload: &AppFeedCardPayload,
    ) -> PortResult<LarkSendOutcome> {
        let path = format!(
            "/open-apis/im/v2/app_feed_card?user_id_type={}",
            user_id_type_str(user_id_type)?
        );
        let body = json!({
            "app_feed_card": app_feed_card_json(payload, true),
            "user_ids": [user_id],
        });
        let data = self.call(reqwest::Method::POST, &path, Some(&body)).await?;
        Ok(LarkSendOutcome {
            message_id: None,
            app_feed_biz_id: data
                .get("biz_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| payload.biz_id.clone()),
            failed_cards: parse_failed_cards(&data),
        })
    }

    async fn update_app_feed_card(
        &self,
        user_id: &str,
        user_id_type: ReceiveIdType,
        payload: &AppFeedCardPayload,
    ) -> PortResult<()> {
        let path = format!(
            "/open-apis/im/v2/app_feed_card/batch?user_id_type={}",
            user_id_type_str(user_id_type)?
        );
        let body = json!({
            "feed_cards": [{
                "app_feed_card": app_feed_card_json(payload, true),
                "user_id": user_id,
                "update_fields": FEED_UPDATE_FIELDS,
            }]
        });
        let data = self.call(reqwest::Method::PUT, &path, Some(&body)).await?;
        let failed = parse_failed_cards(&data);
        if !failed.is_empty() {
            return Err(PortError::Lark(LarkError::new(
                -1,
                format!("更新应用消息流卡片失败: {failed:?}"),
            )));
        }
        Ok(())
    }

    async fn delete_app_feed_card(
        &self,
        user_id: &str,
        user_id_type: ReceiveIdType,
        biz_id: &str,
    ) -> PortResult<()> {
        let path = format!(
            "/open-apis/im/v2/app_feed_card/batch?user_id_type={}",
            user_id_type_str(user_id_type)?
        );
        let body = json!({
            "feed_cards": [{ "biz_id": biz_id, "user_id": user_id }]
        });
        let data = self
            .call(reqwest::Method::DELETE, &path, Some(&body))
            .await?;
        let failed = parse_failed_cards(&data);
        if !failed.is_empty() {
            return Err(PortError::Lark(LarkError::new(
                -1,
                format!("删除应用消息流卡片失败: {failed:?}"),
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alkh_domain::template::{
        FeedButtonType, RenderedFeedButton, RenderedStatusLabel, StatusLabelType,
    };

    fn payload() -> AppFeedCardPayload {
        AppFeedCardPayload {
            biz_id: Some("alkh-abc".into()),
            title: "标题".into(),
            preview: "预览".into(),
            status_label: Some(RenderedStatusLabel {
                text: "严重".into(),
                label_type: StatusLabelType::Danger,
            }),
            link: Some("https://example.com".into()),
            buttons: vec![RenderedFeedButton {
                text: "查看详情".into(),
                url: "https://example.com".into(),
                button_type: FeedButtonType::Primary,
            }],
            time_sensitive: true,
            close_notify: false,
        }
    }

    #[test]
    fn app_feed_json_matches_documented_shape() {
        let v = app_feed_card_json(&payload(), true);
        assert_eq!(v["biz_id"], "alkh-abc");
        assert_eq!(v["title"], "标题");
        assert_eq!(v["status_label"]["text"], "严重");
        assert_eq!(v["status_label"]["type"], "danger");
        assert_eq!(v["link"]["link"], "https://example.com");
        assert_eq!(v["buttons"]["buttons"][0]["action_type"], "url_page");
        assert_eq!(v["buttons"]["buttons"][0]["text"]["text"], "查看详情");
        assert_eq!(
            v["buttons"]["buttons"][0]["multi_url"]["url"],
            "https://example.com"
        );
        assert_eq!(v["time_sensitive"], true);
        assert_eq!(v["notify"]["close_notify"], false);
    }

    #[test]
    fn app_feed_json_omits_optional_fields() {
        let mut p = payload();
        p.status_label = None;
        p.link = None;
        p.buttons.clear();
        let v = app_feed_card_json(&p, true);
        assert!(v.get("status_label").is_none());
        assert!(v.get("link").is_none());
        assert!(v.get("buttons").is_none());
    }

    #[test]
    fn feed_update_fields_cover_documented_enum() {
        for f in FEED_UPDATE_FIELDS {
            assert!(
                matches!(
                    f,
                    "1" | "2" | "3" | "10" | "11" | "12" | "13" | "101" | "102" | "103"
                ),
                "非法 update_fields: {f}"
            );
        }
        assert!(FEED_UPDATE_FIELDS.contains(&"13"), "必须支持即时提醒更新");
    }

    #[test]
    fn user_id_type_rejects_chat_and_email() {
        assert_eq!(user_id_type_str(ReceiveIdType::OpenId).unwrap(), "open_id");
        assert_eq!(user_id_type_str(ReceiveIdType::UserId).unwrap(), "user_id");
        assert!(user_id_type_str(ReceiveIdType::ChatId).is_err());
        assert!(user_id_type_str(ReceiveIdType::Email).is_err());
    }

    #[test]
    fn token_errors_detected() {
        assert!(is_token_error(99991663));
        assert!(!is_token_error(0));
    }

    #[test]
    fn failed_cards_parsed() {
        let data = json!({
            "failed_cards": [
                {"biz_id": "b1", "user_id": "ou_1", "reason": 4},
                {"biz_id": "b2", "user_id": "ou_2", "reason": "NO_PERMISSION"}
            ]
        });
        let cards = parse_failed_cards(&data);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].reason.as_deref(), Some("4"));
        assert_eq!(cards[1].reason.as_deref(), Some("NO_PERMISSION"));
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert!(backoff(1) < backoff(2));
        assert_eq!(backoff(5), backoff(9));
    }

    #[test]
    fn credential_fingerprint_changes_with_secret_and_app() {
        let base = LarkCredential {
            app_id: "cli_a".into(),
            app_secret: "s1".into(),
            base_url: "https://open.feishu.cn".into(),
        };
        let same = LarkCredential {
            app_secret: "s1".into(),
            ..base.clone()
        };
        let rotated = LarkCredential {
            app_secret: "s2".into(),
            ..base.clone()
        };
        let other_app = LarkCredential {
            app_id: "cli_b".into(),
            ..base.clone()
        };
        let other_host = LarkCredential {
            base_url: "https://open.larksuite.com".into(),
            ..base.clone()
        };

        assert_eq!(credential_fingerprint(&base), credential_fingerprint(&same));
        assert_ne!(
            credential_fingerprint(&base),
            credential_fingerprint(&rotated)
        );
        assert_ne!(
            credential_fingerprint(&base),
            credential_fingerprint(&other_app)
        );
        assert_ne!(
            credential_fingerprint(&base),
            credential_fingerprint(&other_host)
        );
        // 指纹里不能出现明文密钥
        assert!(!credential_fingerprint(&base).contains("s1"));
    }
}
