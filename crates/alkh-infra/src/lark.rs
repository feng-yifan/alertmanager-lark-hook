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
//! | Pin 消息 | `POST /open-apis/im/v1/pins` |
//! | 取消 Pin | `DELETE /open-apis/im/v1/pins/:message_id` |

use std::sync::Arc;
use std::time::{Duration, Instant};

use alkh_domain::channel::ReceiveIdType;
use alkh_domain::error::{LarkError, PortError, PortResult};
use alkh_domain::ports::{LarkChat, LarkCredential, LarkCredentials, LarkGateway, LarkSendOutcome};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use tokio::sync::RwLock;

/// 提前 5 分钟刷新 token（飞书 token 有效期约 7200s）。
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;
/// 单个 HTTP 请求的最大重试次数（限频 / 5xx / 网络错误）。
const MAX_ATTEMPTS: u32 = 3;

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
                message: with_remediation(
                    code,
                    if msg.is_empty() {
                        format!("HTTP {status}")
                    } else {
                        format!("{msg} (http {status})")
                    },
                ),
                log_id,
                http_status: Some(status),
                retryable,
            }));
        }
    }
}

/// 常见错误码 → 可执行的中文提示。
///
/// 飞书的 `msg` 是英文，只说「哪里错了」不说「怎么修」；而这些错误码几乎全是
/// **开放平台侧的配置问题**，本服务重试多少次都没用。提示写进 `last_error` 后，
/// 用户在「投递记录」里就能直接看到该做什么，不必再去翻文档。
fn remediation_hint(code: i64) -> Option<&'static str> {
    match code {
        230002 => Some("机器人不在目标群：请把应用机器人拉进该群，并确认群允许机器人发言"),
        230006 => Some(
            "应用未开启「机器人」能力（或开启后没有发布版本）：飞书开放平台 →「添加应用能力」→ 机器人，\
             再「版本管理与发布」→ 创建版本并发布",
        ),
        230013 => Some("用户不在应用可用范围内：在「版本管理与发布 → 可用范围」中把该用户/部门加进去"),
        99991672 => Some("缺少接口权限（scope）：按错误提示的链接申请对应权限，然后创建版本并发布"),
        200380 => Some("卡片模板不存在或尚未发布"),
        _ => None,
    }
}

fn with_remediation(code: i64, message: String) -> String {
    match remediation_hint(code) {
        Some(hint) => format!("{message}；{hint}"),
        None => message,
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

/// 数据包装成 JSON 字符串（飞书要求 `content` / `data` 为转义后的字符串）。
fn as_json_string(v: &Value) -> PortResult<String> {
    serde_json::to_string(v).map_err(|e| PortError::Internal(format!("JSON 序列化失败: {e}")))
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

    async fn pin_message(&self, message_id: &str) -> PortResult<()> {
        let body = json!({ "message_id": message_id });
        self.call(reqwest::Method::POST, "/open-apis/im/v1/pins", Some(&body))
            .await?;
        Ok(())
    }

    async fn unpin_message(&self, message_id: &str) -> PortResult<()> {
        let path = format!("/open-apis/im/v1/pins/{message_id}");
        self.call(reqwest::Method::DELETE, &path, None).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_errors_detected() {
        assert!(is_token_error(99991663));
        assert!(!is_token_error(0));
    }

    /// 「点重试还是收不到」的头号原因都是控制台配置问题：错误里必须带上怎么修，
    /// 否则用户只能看到一个英文 `msg`（如 `Bot ability is not activated`）。
    #[test]
    fn known_error_codes_carry_actionable_hints() {
        for code in [230002, 230006, 230013, 99991672, 200380] {
            let hint = remediation_hint(code).unwrap_or_else(|| panic!("{code} 应有修复提示"));
            assert!(!hint.is_empty());
        }
        // 230006：机器人能力未开启/未发布 —— 提示里必须指明「发布」这一步。
        let hint = remediation_hint(230006).unwrap();
        assert!(hint.contains("机器人"), "{hint}");
        assert!(hint.contains("发布"), "{hint}");

        // 未知错误码不硬编提示，保留飞书原文。
        assert!(remediation_hint(123456).is_none());
        assert_eq!(with_remediation(123456, "raw".into()), "raw");
        assert_eq!(
            with_remediation(230006, "Bot ability is not activated. (http 400)".into()),
            format!(
                "Bot ability is not activated. (http 400)；{}",
                remediation_hint(230006).unwrap()
            )
        );
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
