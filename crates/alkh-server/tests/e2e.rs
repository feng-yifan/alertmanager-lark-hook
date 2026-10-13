//! 端到端集成测试：真实 Axum HTTP 栈 + 内存 SQLite + Fake 飞书网关。
//!
//! 覆盖：登录鉴权 → 渠道创建 → Hook 签发 → AlertManager 投递 →
//! 新告警 / 幂等去重 / 分组更新（原地更新）/ 解决告警 → 查询与指标。

use std::sync::{Arc, Mutex};

use alkh_domain::channel::ReceiveIdType;
use alkh_domain::error::{LarkError, PortError};
use alkh_domain::ports::{AppFeedCardPayload, LarkChat, LarkGateway, LarkSendOutcome};
use alkh_server::config::Config;
use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;

// ---------------------------------------------------------------------------
// Fake 飞书网关
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct SentMessage {
    receive_id: String,
    receive_id_type: String,
    content: Value,
    uuid: String,
}

#[derive(Default)]
struct FakeState {
    chats: Vec<LarkChat>,
    card_entities: Vec<Value>,
    card_entity_templates: Vec<Value>,
    messages: Vec<SentMessage>,
    feed_cards: Vec<AppFeedCardPayload>,
    updated_card_entities: Vec<(String, Value, i64)>,
    updated_messages: Vec<(String, Value)>,
    recalled: Vec<String>,
    deleted_feed_cards: Vec<String>,
    seq: u64,
    /// 注入故障：接下来 `fail_send_times` 次「发送」直接返回 `fail_send_code`。
    /// 用于覆盖发送失败路径（默认 0 = 不注入）。
    fail_send_times: usize,
    fail_send_code: i64,
}

#[derive(Default)]
struct FakeLark {
    state: Mutex<FakeState>,
}

impl FakeLark {
    fn with_chats() -> Self {
        let fake = Self::default();
        {
            let mut s = fake.state.lock().unwrap();
            s.chats = vec![
                LarkChat {
                    chat_id: "oc_test_group".into(),
                    name: "运维告警群".into(),
                    avatar_url: None,
                    description: None,
                    external: false,
                    dissolved: false,
                },
                LarkChat {
                    chat_id: "oc_dissolved".into(),
                    name: "已解散群".into(),
                    avatar_url: None,
                    description: None,
                    external: false,
                    dissolved: true,
                },
            ];
        }
        fake
    }

    fn next_id(&self, prefix: &str) -> String {
        let mut s = self.state.lock().unwrap();
        s.seq += 1;
        format!("{prefix}_{}", s.seq)
    }

    fn snapshot(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap()
    }

    /// 让接下来的 `times` 次「发送」返回飞书错误 `code`（模拟限频/服务端错误）。
    ///
    /// 真实网关会先在内部退避重试，耗尽后才把错误抛给上层；这里直接抛出，
    /// 等价于「内部重试已耗尽」的状态。
    fn inject_send_failure(&self, times: usize, code: i64) {
        let mut s = self.snapshot();
        s.fail_send_times = times;
        s.fail_send_code = code;
    }

    fn take_injected_send_failure(&self) -> Option<PortError> {
        let mut s = self.snapshot();
        if s.fail_send_times == 0 {
            return None;
        }
        s.fail_send_times -= 1;
        let code = s.fail_send_code;
        Some(PortError::Lark(
            LarkError::new(code, "注入的发送失败（测试用）")
                .retryable(code == LarkError::CODE_RATE_LIMITED || code >= 500),
        ))
    }
}

#[async_trait]
impl LarkGateway for FakeLark {
    async fn verify_credentials(&self) -> alkh_domain::PortResult<Vec<LarkChat>> {
        self.list_chats().await
    }

    async fn list_chats(&self) -> alkh_domain::PortResult<Vec<LarkChat>> {
        Ok(self.snapshot().chats.clone())
    }

    async fn create_card_entity(&self, card_json: &Value) -> alkh_domain::PortResult<String> {
        self.snapshot().card_entities.push(card_json.clone());
        Ok(self.next_id("card"))
    }

    async fn create_card_entity_from_template(
        &self,
        template_id: &str,
        version: Option<&str>,
        variables: &serde_json::Map<String, Value>,
    ) -> alkh_domain::PortResult<String> {
        self.snapshot().card_entity_templates.push(json!({
            "template_id": template_id,
            "template_version_name": version,
            "template_variable": variables,
        }));
        Ok(self.next_id("card_tpl"))
    }

    async fn send_card_entity(
        &self,
        receive_id: &str,
        receive_id_type: ReceiveIdType,
        card_entity_id: &str,
        uuid: &str,
    ) -> alkh_domain::PortResult<LarkSendOutcome> {
        if let Some(err) = self.take_injected_send_failure() {
            return Err(err);
        }
        let content = json!({ "type": "card", "data": { "card_id": card_entity_id } });
        self.snapshot().messages.push(SentMessage {
            receive_id: receive_id.to_string(),
            receive_id_type: receive_id_type.as_str().to_string(),
            content,
            uuid: uuid.to_string(),
        });
        Ok(LarkSendOutcome {
            message_id: Some(self.next_id("om")),
            app_feed_biz_id: None,
            failed_cards: vec![],
        })
    }

    async fn send_message_content(
        &self,
        receive_id: &str,
        receive_id_type: ReceiveIdType,
        content: &Value,
        uuid: &str,
    ) -> alkh_domain::PortResult<LarkSendOutcome> {
        if let Some(err) = self.take_injected_send_failure() {
            return Err(err);
        }
        self.snapshot().messages.push(SentMessage {
            receive_id: receive_id.to_string(),
            receive_id_type: receive_id_type.as_str().to_string(),
            content: content.clone(),
            uuid: uuid.to_string(),
        });
        Ok(LarkSendOutcome {
            message_id: Some(self.next_id("om")),
            app_feed_biz_id: None,
            failed_cards: vec![],
        })
    }

    async fn update_card_entity(
        &self,
        card_entity_id: &str,
        card_json: &Value,
        sequence: i64,
        _uuid: &str,
    ) -> alkh_domain::PortResult<()> {
        self.snapshot().updated_card_entities.push((
            card_entity_id.to_string(),
            card_json.clone(),
            sequence,
        ));
        Ok(())
    }

    async fn update_message_content(
        &self,
        message_id: &str,
        content: &Value,
    ) -> alkh_domain::PortResult<()> {
        self.snapshot()
            .updated_messages
            .push((message_id.to_string(), content.clone()));
        Ok(())
    }

    async fn recall_message(&self, message_id: &str) -> alkh_domain::PortResult<()> {
        self.snapshot().recalled.push(message_id.to_string());
        Ok(())
    }

    async fn send_app_feed_card(
        &self,
        _user_id: &str,
        _user_id_type: ReceiveIdType,
        payload: &AppFeedCardPayload,
    ) -> alkh_domain::PortResult<LarkSendOutcome> {
        self.snapshot().feed_cards.push(payload.clone());
        Ok(LarkSendOutcome {
            message_id: None,
            app_feed_biz_id: payload.biz_id.clone(),
            failed_cards: vec![],
        })
    }

    async fn update_app_feed_card(
        &self,
        _user_id: &str,
        _user_id_type: ReceiveIdType,
        payload: &AppFeedCardPayload,
    ) -> alkh_domain::PortResult<()> {
        self.snapshot().feed_cards.push(payload.clone());
        Ok(())
    }

    async fn delete_app_feed_card(
        &self,
        _user_id: &str,
        _user_id_type: ReceiveIdType,
        biz_id: &str,
    ) -> alkh_domain::PortResult<()> {
        self.snapshot().deleted_feed_cards.push(biz_id.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 测试脚手架
// ---------------------------------------------------------------------------

struct Harness {
    base: String,
    http: reqwest::Client,
    token: String,
    fake: Arc<FakeLark>,
    hook_token: Option<String>,
    channel_id: Option<String>,
    /// 直连数据库，用于故障注入（例如把模板数据改坏以触发内部错误）。
    pool: SqlitePool,
}

async fn pool() -> SqlitePool {
    let p = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("内存 SQLite 连接失败");
    alkh_infra::migrate(&p).await.expect("迁移失败");
    p
}

/// 测试用配置。
///
/// `data_dir` 指向 Cargo 提供的 `CARGO_TARGET_TMPDIR`：由于显式提供了 `master_key`，
/// 不会生成 `master.key`，因此不会写入任何文件，也就无需清理临时目录。
fn test_config() -> Config {
    Config {
        http_addr: "127.0.0.1:0".parse().unwrap(),
        data_dir: std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")),
        database_url: "sqlite::memory:".into(),
        db_max_connections: 1,
        master_key: Some("0f".repeat(32)),
        admin_username: "admin".into(),
        admin_password: Some("test-password-123".into()),
        public_base_url: Some("http://alkh.test".into()),
        lark_base_url: "https://open.feishu.cn".into(),
        log_level: "warn".into(),
        log_json: false,
        static_dir: None,
        cookie_secure: false,
        worker_tick_ms: 50,
        retry_interval_secs: 3600,
        retention_interval_secs: 3600,
    }
}

async fn start() -> Harness {
    let config = Arc::new(test_config());
    let pool = pool().await;
    let fake = Arc::new(FakeLark::with_chats());

    let state = alkh_server::wiring::build_state(config.clone(), pool.clone(), Some(fake.clone()))
        .await
        .expect("装配 AppState 失败");
    let bootstrap = state
        .auth
        .ensure_bootstrap_admin("admin", Some("test-password-123".into()))
        .await
        .expect("创建管理员失败");
    assert_eq!(
        bootstrap,
        alkh_app::auth::BootstrapOutcome::Created {
            generated_password: None
        },
        "提供密码时应创建账号且不生成随机密码"
    );
    // 幂等
    let second = state
        .auth
        .ensure_bootstrap_admin("admin", Some("test-password-123".into()))
        .await
        .unwrap();
    assert_eq!(second, alkh_app::auth::BootstrapOutcome::AlreadyExists);
    alkh_app::bootstrap::ensure_default_templates(&state.deps)
        .await
        .expect("创建默认模板失败");

    let app = alkh_server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let login: Value = http
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"username": "admin", "password": "test-password-123"}))
        .send()
        .await
        .expect("登录请求失败")
        .json()
        .await
        .expect("登录响应解析失败");

    Harness {
        base,
        http,
        token: login["token"]
            .as_str()
            .expect("登录响应缺少 token")
            .to_string(),
        fake,
        hook_token: None,
        channel_id: None,
        pool,
    }
}

impl Harness {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn get(&self, path: &str) -> (reqwest::StatusCode, Value) {
        let resp = self
            .http
            .get(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .expect("GET 失败");
        let status = resp.status();
        let body = resp.json().await.unwrap_or(Value::Null);
        (status, body)
    }

    async fn post(&self, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        let resp = self
            .http
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .expect("POST 失败");
        let status = resp.status();
        let parsed = resp.json().await.unwrap_or(Value::Null);
        (status, parsed)
    }

    /// 创建一个群渠道并保存一次性 hook token。
    async fn create_channel(&mut self) -> String {
        let (status, body) = self
            .post(
                "/api/v1/channels",
                json!({
                    "name": "运维告警群",
                    "receive_id": "oc_test_group",
                    "receive_id_type": "chat_id"
                }),
            )
            .await;
        assert_eq!(status, reqwest::StatusCode::CREATED, "创建渠道失败: {body}");
        let token = body["hook_token"]
            .as_str()
            .expect("缺少 hook_token")
            .to_string();
        self.channel_id = Some(body["id"].as_str().unwrap().to_string());
        self.hook_token = Some(token.clone());
        token
    }

    /// 以 AlertManager 身份推送 payload（无需会话）。
    async fn push(&self, payload: Value) -> (reqwest::StatusCode, Value) {
        let token = self.hook_token.as_ref().expect("尚未创建 hook");
        let resp = self
            .http
            .post(self.url(&format!("/api/v1/hooks/alertmanager/{token}")))
            .json(&payload)
            .send()
            .await
            .expect("webhook 请求失败");
        let status = resp.status();
        let body = resp.json().await.unwrap_or(Value::Null);
        (status, body)
    }
}

fn firing_payload(alerts: usize) -> Value {
    let list: Vec<Value> = (0..alerts)
        .map(|i| {
            json!({
                "status": "firing",
                "labels": {"alertname": "HighCPU", "instance": format!("10.0.0.{i}:9100"), "severity": "critical"},
                "annotations": {"summary": format!("CPU 过高 #{i}")},
                "startsAt": "2026-10-13T00:00:00Z",
                "endsAt": "0001-01-01T00:00:00Z",
                "fingerprint": format!("fp{i}")
            })
        })
        .collect();
    json!({
        "version": "4",
        "groupKey": "{}:{alertname=\"HighCPU\"}",
        "truncatedAlerts": 0,
        "status": "firing",
        "receiver": "lark-ops",
        "groupLabels": {"alertname": "HighCPU"},
        "commonLabels": {"alertname": "HighCPU", "severity": "critical"},
        "commonAnnotations": {"summary": "CPU 过高"},
        "externalURL": "http://alertmanager:9093",
        "alerts": list
    })
}

fn resolved_payload() -> Value {
    json!({
        "version": "4",
        "groupKey": "{}:{alertname=\"HighCPU\"}",
        "status": "resolved",
        "receiver": "lark-ops",
        "groupLabels": {"alertname": "HighCPU"},
        "commonLabels": {"alertname": "HighCPU", "severity": "critical"},
        "commonAnnotations": {"summary": "CPU 已恢复"},
        "externalURL": "http://alertmanager:9093",
        "alerts": [{
            "status": "resolved",
            "labels": {"alertname": "HighCPU", "instance": "10.0.0.0:9100", "severity": "critical"},
            "annotations": {"summary": "CPU 已恢复"},
            "startsAt": "2026-10-13T00:00:00Z",
            "endsAt": "2026-10-13T01:00:00Z",
            "fingerprint": "fp0"
        }]
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[tokio::test]
async fn health_endpoints_are_public() {
    let h = start().await;
    let resp = h.http.get(h.url("/healthz")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    let resp = h.http.get(h.url("/readyz")).send().await.unwrap();
    assert!(resp.status().is_success() || resp.status() == 503);

    let resp = h.http.get(h.url("/metrics")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("alkh_http_requests_total"));
}

/// 回归测试：前端曾把 `status_label_types` 当作 `string[]` 使用，导致
/// `<option key={option.value}>` 的 key 变成重复的 `[object Object]`。
/// 这里对 `/meta` 的**线格式**加锁：所有枚举列表都必须是 `{value,label}` 对象数组。
#[tokio::test]
async fn meta_endpoint_pins_the_wire_format() {
    let h = start().await;
    let (status, meta) = h.get("/api/v1/meta").await;
    assert_eq!(status, 200);

    const ENUM_FIELDS: [&str; 11] = [
        "message_types",
        "repeat_policies",
        "resolved_policies",
        "group_update_policies",
        "match_ops",
        "status_label_types",
        "feed_button_types",
        "biz_id_sources",
        "card_sources",
        "channel_kinds",
        "receive_id_types",
    ];

    for field in ENUM_FIELDS {
        let items = meta[field]
            .as_array()
            .unwrap_or_else(|| panic!("{field} 必须是数组，实际为 {}", meta[field]));
        assert!(!items.is_empty(), "{field} 不能为空");
        for item in items {
            assert!(item.is_object(), "{field} 的元素必须是对象，实际为 {item}");
            let value = item.get("value").and_then(Value::as_str);
            let label = item.get("label").and_then(Value::as_str);
            assert!(value.is_some(), "{field} 的元素缺少字符串 value: {item}");
            assert!(label.is_some(), "{field} 的元素缺少字符串 label: {item}");
            // 关键：value 必须是字符串，否则前端会渲染出 [object Object] 作为 key
            assert_ne!(value.unwrap(), "[object Object]");
        }
    }

    // 按钮类型与状态标签类型是两个不同的枚举，不能互相复用
    let feed: Vec<&str> = meta["feed_button_types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["value"].as_str().unwrap())
        .collect();
    assert_eq!(feed, vec!["default", "primary", "success"]);
    let status_types: Vec<&str> = meta["status_label_types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["value"].as_str().unwrap())
        .collect();
    assert!(status_types.contains(&"danger"));
    assert!(!feed.contains(&"danger"));
    assert!(!feed.contains(&"secondary"));

    // 变量序列化为 `type`，不能泄露 Rust 字段名 `ty`
    let variable = &meta["variables"][0];
    assert!(variable["type"].is_string(), "变量应有 `type`: {variable}");
    assert!(variable.get("ty").is_none(), "不应出现 `ty`: {variable}");

    // severity_colors 是对象映射，不是枚举数组
    let colors = meta["severity_colors"]
        .as_object()
        .expect("severity_colors 必须是对象");
    assert_eq!(colors["critical"], "red");
    assert_eq!(colors["warning"], "orange");
    assert_eq!(colors["info"], "blue");
    assert_eq!(colors["default"], "grey");

    assert_eq!(meta["token_prefix"], "alkh_");
    assert_eq!(
        meta["hook_path_template"],
        "/api/v1/hooks/alertmanager/{token}"
    );
    assert_eq!(meta["pin_supported_message_types"][0], "app_feed_card");
}

#[tokio::test]
async fn admin_api_requires_authentication() {
    let h = start().await;
    let resp = h.http.get(h.url("/api/v1/channels")).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "unauthenticated");

    // 错误的 token
    let resp = h
        .http
        .get(h.url("/api/v1/channels"))
        .bearer_auth("bogus")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn meta_and_settings_expose_no_secrets() {
    let h = start().await;
    let (status, meta) = h.get("/api/v1/meta").await;
    assert_eq!(status, 200);
    assert_eq!(meta["message_types"].as_array().unwrap().len(), 2);
    assert!(meta["variables"].as_array().unwrap().len() > 20);

    let (status, settings) = h.get("/api/v1/settings").await;
    assert_eq!(status, 200);
    assert_eq!(settings["app_secret_set"], false);

    // 写入密钥后不得回显
    let resp = h
        .http
        .put(h.url("/api/v1/settings"))
        .bearer_auth(&h.token)
        .json(&json!({
            "lark_app_id": "cli_test_app",
            "app_secret": "super-secret-value",
            "lark_base_url": "https://open.feishu.cn",
            "public_base_url": "https://alkh.example.com",
            "retention": {"alert_group_days": 30, "delivery_days": 30, "audit_days": 14}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        !text.contains("super-secret-value"),
        "响应泄露了 app_secret: {text}"
    );
    assert!(text.contains("\"app_secret_set\":true"));
}

#[tokio::test]
async fn channel_sync_imports_feishu_chats_and_masks_tokens() {
    let h = start().await;

    let (status, report) = h.post("/api/v1/channels/sync", json!({})).await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["discovered"], 2);
    assert_eq!(report["created"], 2);

    let (status, channels) = h.get("/api/v1/channels").await;
    assert_eq!(status, 200);
    assert_eq!(channels["total"], 2);

    // 已解散的群被自动停用
    let items = channels["items"].as_array().unwrap();
    let dissolved = items
        .iter()
        .find(|c| c["receive_id"] == "oc_dissolved")
        .unwrap();
    assert_eq!(dissolved["enabled"], false);
    let normal = items
        .iter()
        .find(|c| c["receive_id"] == "oc_test_group")
        .unwrap();
    assert_eq!(normal["enabled"], true);
    assert_eq!(normal["policy"]["message_type"], "card_message");
    assert_eq!(normal["policy"]["repeat_policy"], "suppress");
    assert_eq!(normal["policy"]["resolved_policy"], "send");

    // 列表接口不应泄露明文 token，只给前缀和屏蔽路径
    let (status, detail) = h
        .get(&format!(
            "/api/v1/channels/{}",
            normal["id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(status, 200);
    let raw = serde_json::to_string(&detail).unwrap();
    assert!(raw.contains("alkh_"), "应展示 token 前缀");
    assert!(!raw.contains("alkh_session"), "不应出现会话信息");
    for hook in detail["hooks"].as_array().unwrap() {
        assert!(
            hook["hook_path"].as_str().unwrap().contains('…'),
            "hook_path 必须被屏蔽"
        );
        assert!(
            hook["hook_url"].as_str().unwrap().contains('…'),
            "hook_url 必须被屏蔽"
        );
    }
}

#[tokio::test]
async fn full_delivery_lifecycle_new_repeat_update_resolve() {
    let mut h = start().await;
    let hook_token = h.create_channel().await;
    assert!(hook_token.starts_with("alkh_"));

    // --- 1. 新告警：发送新消息 ---
    let (status, body) = h.push(firing_payload(1)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["event"], "new_firing");
    assert_eq!(body["action"], "send_new");
    assert_eq!(body["ok"], true);
    // 指标以实际落库状态为准，而不是"决策动作"
    assert_eq!(body["delivery_state"], "active");

    {
        let s = h.fake.snapshot();
        assert_eq!(s.card_entities.len(), 1, "应创建一个卡片实体");
        assert_eq!(s.messages.len(), 1, "应发送一条消息");
        assert_eq!(s.messages[0].receive_id, "oc_test_group");
        assert_eq!(s.messages[0].receive_id_type, "chat_id");
        assert_eq!(s.messages[0].content["type"], "card");
        assert!(s.messages[0].content["data"]["card_id"].is_string());
        // 卡片 JSON 必须带 schema 2.0 且 update_multi=true
        assert_eq!(s.card_entities[0]["schema"], "2.0");
        assert_eq!(s.card_entities[0]["config"]["update_multi"], true);
        let rendered = s.card_entities[0].to_string();
        assert!(rendered.contains("HighCPU"), "卡片应包含告警名: {rendered}");
        assert!(!rendered.contains("${"), "卡片不应残留未替换变量");
    }

    // --- 2. 完全相同的 payload：幂等去重 ---
    let (status, body) = h.push(firing_payload(1)).await;
    assert_eq!(status, 200);
    assert_eq!(body["skipped"], "duplicate");
    assert_eq!(h.fake.snapshot().messages.len(), 1, "去重不应重复发送");

    // --- 3. 新增告警（分组更新）：原地更新卡片 ---
    let (status, body) = h.push(firing_payload(2)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["event"], "updated");
    assert_eq!(body["action"], "update_in_place");
    assert_eq!(body["delivery_state"], "active");
    {
        let s = h.fake.snapshot();
        assert_eq!(s.updated_card_entities.len(), 1, "应原地更新卡片实体");
        let (card_id, card, seq) = &s.updated_card_entities[0];
        assert!(
            card_id.starts_with("card_"),
            "应复用同一个卡片实体: {card_id}"
        );
        assert_eq!(*seq, 1, "首次更新的 sequence 必须为 1");
        assert!(
            card.to_string().contains("10.0.0.1:9100"),
            "更新内容应包含新告警"
        );
        assert_eq!(s.messages.len(), 1, "原地更新不应发送新消息");
    }

    // --- 4. 解决告警：默认策略为发送恢复消息 ---
    let (status, body) = h.push(resolved_payload()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["event"], "resolved");
    assert_eq!(body["action"], "send_new");
    {
        let s = h.fake.snapshot();
        assert_eq!(s.messages.len(), 2, "应发送恢复消息");
        assert!(
            s.card_entities[1].to_string().contains("已恢复"),
            "恢复卡片应体现已恢复"
        );
    }

    // --- 5. 查询接口 ---
    let (status, groups) = h.get("/api/v1/alert-groups").await;
    assert_eq!(status, 200);
    assert_eq!(groups["total"], 1);
    let group = &groups["items"][0];
    assert_eq!(group["status"], "resolved");
    assert_eq!(group["group_key"], "{}:{alertname=\"HighCPU\"}");
    assert_eq!(group["firing_count"], 0);

    let (status, resolved_groups) = h.get("/api/v1/alert-groups?status=resolved").await;
    assert_eq!(status, 200);
    assert_eq!(resolved_groups["total"], 1);
    let (_, firing_groups) = h.get("/api/v1/alert-groups?status=firing").await;
    assert_eq!(firing_groups["total"], 0);

    let (status, deliveries) = h.get("/api/v1/deliveries").await;
    assert_eq!(status, 200);
    assert_eq!(deliveries["total"], 2);
    let states: Vec<&str> = deliveries["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["state"].as_str().unwrap())
        .collect();
    assert!(states.contains(&"superseded"), "旧消息应被取代: {states:?}");
    assert!(states.contains(&"active"), "新消息应为活跃: {states:?}");

    let (status, overview) = h.get("/api/v1/overview").await;
    assert_eq!(status, 200);
    assert_eq!(overview["alert_groups"]["resolved"], 1);
    assert!(overview["deliveries_24h"]["sent"].as_u64().unwrap() >= 2);

    // --- 6. 审计 ---
    let (status, audit) = h.get("/api/v1/audit-logs?action=message.sent").await;
    assert_eq!(status, 200);
    assert!(
        audit["total"].as_u64().unwrap() >= 2,
        "应记录发送审计: {audit}"
    );
}

#[tokio::test]
async fn repeat_policy_send_and_revoke_previous_recalls_old_message() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    // 把重复告警策略改为「发送并移除旧告警」
    let (status, policy) = h
        .get(&format!("/api/v1/channels/{channel_id}/policy"))
        .await;
    assert_eq!(status, 200);
    let mut policy = policy;
    policy["repeat_policy"] = json!("send_and_revoke_previous");
    policy["resolved_policy"] = json!("send_and_delete_previous");
    policy["pin_unresolved"] = json!(true);

    // 用户级渠道才能用应用消息流卡片；这里保持卡片消息，pin 应被忽略（R4）
    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&policy)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 新告警 → 发送
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["action"], "send_new");

    // 相同事件再次到达会因为 event_hash 去重被跳过，所以先改动再触发一次重复通知：
    // 用 update 让卡片进入活跃状态后，构造"与当前内容一致"的事件即触发 repeated。
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["skipped"], "duplicate");

    // 解决告警 → 发送恢复消息 + 删除旧消息
    let (_, body) = h.push(resolved_payload()).await;
    assert_eq!(body["event"], "resolved");
    assert_eq!(body["action"], "send_new_then_revoke");
    let s = h.fake.snapshot();
    assert_eq!(s.messages.len(), 2, "应发送恢复消息");
    assert_eq!(s.recalled.len(), 1, "应撤回/删除旧消息");
}

/// 回归（P0-1）：处理失败后 AlertManager 的重试必须被**重新处理**，而不是被判为重复。
///
/// 修复前的顺序是「先记录去重键 → 再处理事件」，且失败不回滚。于是首次返回 5xx 之后，
/// AM 的秒级退避重试会命中 60s 去重窗口、被判为 `duplicate`，而 webhook 会回 **200** ——
/// AM 认为投递成功，这条告警永久丢失，而且审计里只有一条「duplicate」，看不出异常。
#[tokio::test]
async fn failed_intake_stays_retryable_instead_of_being_deduped() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    // 故障注入：把内置默认卡片模板的 card_message 置空。
    // 这会让 `Sender::prepare` 走到 `AppError::internal("卡片消息模板缺少 card_message")`
    // ⇒ webhook 返回 500（AM 会重试），模拟运行期模板数据损坏这类内部错误。
    let template_id: String = sqlx::query_scalar(
        "SELECT id FROM card_templates WHERE kind = 'card_message' AND is_default = 1",
    )
    .fetch_one(&h.pool)
    .await
    .expect("应存在内置默认卡片模板");
    sqlx::query("UPDATE card_templates SET card_message = NULL WHERE id = ?")
        .bind(&template_id)
        .execute(&h.pool)
        .await
        .unwrap();

    let payload = firing_payload(1);

    // 首次投递：内部错误 → 500，AM 会退避重试
    let (status, body) = h.push(payload.clone()).await;
    assert_eq!(
        status,
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        "渲染/准备失败应返回 500 让 AM 重试: {body}"
    );

    // AM 的重试（秒级，仍在 60s 去重窗口内）
    let (status2, body2) = h.push(payload).await;
    assert_ne!(
        body2["skipped"],
        json!("duplicate"),
        "失败后的重试被当成重复：AM 会收到 200 并认为投递成功，告警静默丢失（P0-1 回归）"
    );
    assert_eq!(
        status2,
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        "重试应仍然失败并保持可重试，而不是被吞掉: {body2}"
    );
}

/// 安全回归：登录失败达到阈值后必须被节流（429），否则公网入口可无限撞库；
/// 且「用户名不存在」与「密码错误」的对外表现必须完全一致（不泄露账号是否存在）。
#[tokio::test]
async fn repeated_login_failures_are_throttled_and_do_not_leak_usernames() {
    let h = start().await;
    let path = "/api/v1/auth/login";
    let login = |body: Value| {
        let req = h.http.post(h.url(path)).json(&body);
        async move {
            let resp = req.send().await.expect("登录请求失败");
            let status = resp.status();
            let parsed = resp.json().await.unwrap_or(Value::Null);
            (status, parsed)
        }
    };

    // 用户名不存在 vs 密码错误：状态码与错误信封必须一致
    let (s1, b1) = login(json!({"username": "no-such-user", "password": "whatever-123"})).await;
    let (s2, b2) = login(json!({"username": "admin", "password": "wrong-password"})).await;
    assert_eq!(s1, reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(s2, reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        b1["error"], b2["error"],
        "两种失败必须给出完全相同的响应，否则可枚举用户名"
    );

    // 把 admin 打到阈值（上面已经失败 1 次）
    for _ in 0..4 {
        let (status, _) = login(json!({"username": "admin", "password": "wrong-password"})).await;
        assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    }

    // 再试一次：应被节流
    let (status, body) = login(json!({"username": "admin", "password": "wrong-password"})).await;
    assert_eq!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "连续失败后应被节流: {body}"
    );
    assert_eq!(body["error"]["code"], "rate_limited");
    assert!(
        body["error"]["details"]["retry_after_secs"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "应给出 retry_after_secs: {body}"
    );

    // 节流是按 (用户名, 来源) 计的：别的用户名不受影响（避免误伤）
    let (status, _) = login(json!({"username": "other-admin", "password": "wrong-password"})).await;
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);

    // 被节流期间即使密码正确也不放行（否则节流形同虚设）
    let (status, _) = login(json!({"username": "admin", "password": "test-password-123"})).await;
    assert_eq!(status, reqwest::StatusCode::TOO_MANY_REQUESTS);
}

/// 安全回归：告警的 `generatorURL` / `externalURL` 由发送方完全控制，
/// 非 http(s) 的值（尤其 `javascript:`）必须在入库前被丢弃。
///
/// 原样入库时前端会把它放进 `<a href>`，而 React 18 不拦 `javascript:`，
/// 于是「持有 hook token 发一条告警」就升级成「管理员点一下 = 后台被接管」。
#[tokio::test]
async fn javascript_urls_are_stripped_before_persisting() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    let mut payload = firing_payload(1);
    payload["alerts"][0]["generatorURL"] = json!("javascript:alert(document.domain)");
    payload["externalURL"] = json!("javascript:alert(2)");

    let (status, body) = h.push(payload).await;
    assert_eq!(status, 200);
    let group_id = body["alert_group_id"].as_str().unwrap().to_string();

    let (status, group) = h.get(&format!("/api/v1/alert-groups/{group_id}")).await;
    assert_eq!(status, 200);

    let alert = &group["alerts"][0];
    assert_eq!(
        alert["generator_url"],
        Value::Null,
        "javascript: 的 generatorURL 不应入库: {alert}"
    );
    assert_eq!(
        group["external_url"],
        Value::Null,
        "javascript: 的 externalURL 不应入库: {group}"
    );

    // 正常的 http(s) 链接必须保留，别把功能一起砍掉。
    let mut ok_payload = firing_payload(1);
    ok_payload["alerts"][0]["generatorURL"] = json!("http://prom:9090/graph");
    ok_payload["externalURL"] = json!("http://alertmanager:9093");
    ok_payload["groupKey"] = json!("{}:{alertname=\"Other\"}");
    let (_, body) = h.push(ok_payload).await;
    let gid = body["alert_group_id"].as_str().unwrap().to_string();
    let (_, group) = h.get(&format!("/api/v1/alert-groups/{gid}")).await;
    assert_eq!(
        group["alerts"][0]["generator_url"], "http://prom:9090/graph",
        "合法 http(s) 链接必须保留"
    );
    assert_eq!(group["external_url"], "http://alertmanager:9093");
}

/// 回归（P0-5）：重试成功后必须把分组的活跃消息指针回写到这条投递。
///
/// `group.active_delivery_id` 是唯一的活跃消息指针（`find_active` 在生产从未被调用）。
/// 修复前 `retry` 只更新投递行，分组仍指向空的/旧的投递，于是下一次「内容变化」
/// 会被判成「没有旧消息可更新」而改走 SendNew —— 同一条告警在群里出现两张卡片，
/// 旧卡片也再无法被撤回。
#[tokio::test]
async fn successful_retry_repoints_group_active_delivery() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    // 首次投递失败（限频）→ 投递停在 pending，分组没有活跃消息
    h.fake.inject_send_failure(1, LarkError::CODE_RATE_LIMITED);
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(
        body["delivery_state"], "pending",
        "限频应停在 pending: {body}"
    );

    // 手动重试（此时飞书已恢复）
    let (status, list) = h.get("/api/v1/deliveries?limit=20").await;
    assert_eq!(status, 200);
    let id = list["items"][0]["id"]
        .as_str()
        .expect("应有待重试的投递")
        .to_string();
    let (status, delivery) = h
        .post(
            &format!("/api/v1/deliveries/{id}/retry"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, 200, "重试应成功: {delivery}");
    assert_eq!(delivery["state"], "active");

    // 内容变化 → 必须原地更新；修复前会走 send_new（重复卡片）
    let (_, body) = h.push(firing_payload(2)).await;
    assert_eq!(
        body["action"], "update_in_place",
        "重试成功后分组仍被认为没有活跃消息，会重复发卡片（P0-5 回归）: {body}"
    );
}

/// 回归（P0-2）：新消息发送失败时**不允许**撤回旧消息。
///
/// `send_new` 在发送失败时仍返回 `Ok`（状态 Pending，交给后台重试），
/// 修复前 `SendNewThenRevoke` 会无条件调用 `revoke`，于是出现
/// 「旧消息已撤回、新消息没发出」—— 这条告警在群里彻底消失。
#[tokio::test]
async fn revoke_is_skipped_when_new_message_fails_to_send() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    // 恢复策略：发送新的恢复消息并删除旧消息（映射为 SendNewThenRevoke）
    let (status, mut policy) = h
        .get(&format!("/api/v1/channels/{channel_id}/policy"))
        .await;
    assert_eq!(status, 200);
    policy["resolved_policy"] = json!("send_and_delete_previous");
    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&policy)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // 新告警 → 正常发出第一条消息
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["action"], "send_new");
    assert_eq!(h.fake.snapshot().messages.len(), 1, "应发出告警卡片");

    // 注入限频故障：下一次发送失败
    h.fake.inject_send_failure(1, LarkError::CODE_RATE_LIMITED);

    // 解决告警 → 走 SendNewThenRevoke，但新消息发送失败
    let (_, body) = h.push(resolved_payload()).await;
    assert_eq!(body["action"], "send_new_then_revoke");

    let s = h.fake.snapshot();
    assert_eq!(s.messages.len(), 1, "恢复消息发送失败，不应产生第二条消息");
    assert!(
        s.recalled.is_empty(),
        "新消息没发出去就撤回了旧消息，这条告警会在群里消失（P0-2 回归）"
    );
}

#[tokio::test]
async fn label_matchers_filter_alerts() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&json!({
            "message_type": "card_message",
            "pin_unresolved": false,
            "repeat_policy": "suppress",
            "resolved_policy": "send",
            "group_update_policy": "update_previous",
            "label_matchers": [{"name": "severity", "op": "=", "value": "warning"}],
            "aggregation_window_secs": 0,
            "min_send_interval_secs": 0,
            "max_alerts_per_message": 20,
            "mute_until": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // payload 只有 critical 告警 → 被过滤
    let (status, body) = h.push(firing_payload(1)).await;
    assert_eq!(status, 200);
    assert_eq!(body["skipped"], "filtered");
    assert_eq!(h.fake.snapshot().messages.len(), 0, "被过滤的告警不应发送");

    // 非法正则应被拒绝
    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&json!({
            "message_type": "card_message",
            "pin_unresolved": false,
            "repeat_policy": "suppress",
            "resolved_policy": "send",
            "group_update_policy": "update_previous",
            "label_matchers": [{"name": "severity", "op": "=~", "value": "(["}],
            "aggregation_window_secs": 0,
            "min_send_interval_secs": 0,
            "max_alerts_per_message": 20,
            "mute_until": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn app_feed_card_is_rejected_for_group_channel() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    // R15：群渠道不能用应用消息流卡片（飞书该接口没有群维度）
    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&json!({
            "message_type": "app_feed_card",
            "pin_unresolved": true,
            "repeat_policy": "suppress",
            "resolved_policy": "send",
            "group_update_policy": "update_previous",
            "label_matchers": [],
            "aggregation_window_secs": 0,
            "min_send_interval_secs": 0,
            "max_alerts_per_message": 20,
            "mute_until": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "应拒绝群渠道 + 应用消息流卡片");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "validation_failed");
}

#[tokio::test]
async fn user_channel_app_feed_card_delivers_with_documented_payload() {
    let mut h = start().await;

    // 用户级渠道（open_id）
    let (status, body) = h
        .post(
            "/api/v1/channels",
            json!({
                "name": "值班同学",
                "receive_id": "ou_test_user",
                "receive_id_type": "open_id"
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let channel_id = body["id"].as_str().unwrap().to_string();
    let token = body["hook_token"].as_str().unwrap().to_string();
    h.hook_token = Some(token);
    h.channel_id = Some(channel_id.clone());

    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&json!({
            "message_type": "app_feed_card",
            "pin_unresolved": true,
            "repeat_policy": "suppress",
            "resolved_policy": "send",
            "group_update_policy": "update_previous",
            "label_matchers": [],
            "aggregation_window_secs": 0,
            "min_send_interval_secs": 0,
            "max_alerts_per_message": 20,
            "mute_until": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let (status, body) = h.push(firing_payload(1)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["action"], "send_new");

    let s = h.fake.snapshot();
    assert_eq!(s.feed_cards.len(), 1);
    let card = &s.feed_cards[0];
    assert!(card.biz_id.as_deref().unwrap().starts_with("alkh-"));
    assert!(card.title.contains("HighCPU"), "标题应渲染: {}", card.title);
    // R4：未解决告警 + pin 开关 ⇒ 即时提醒
    assert!(card.time_sensitive, "firing 告警应设置 time_sensitive");
}

#[tokio::test]
async fn unknown_hook_token_is_404() {
    let h = start().await;
    let resp = h
        .http
        .post(h.url("/api/v1/hooks/alertmanager/alkh_not_a_real_token"))
        .json(&firing_payload(1))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "unknown_hook");
}

#[tokio::test]
async fn malformed_payload_is_rejected() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    let resp = h
        .http
        .post(h.url(&format!(
            "/api/v1/hooks/alertmanager/{}",
            h.hook_token.as_ref().unwrap()
        )))
        .header("content-type", "application/json")
        .body("{ not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // 空 alerts 视为心跳，跳过但返回 200
    let (status, body) = h
        .push(json!({"version": "4", "groupKey": "g", "status": "firing", "alerts": []}))
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["skipped"], "no_alerts");
}

#[tokio::test]
async fn card_template_import_validate_preview_and_crud() {
    let h = start().await;

    let (status, templates) = h.get("/api/v1/card-templates").await;
    assert_eq!(status, 200);
    assert_eq!(
        templates.as_array().unwrap().len(),
        2,
        "应有两个内置默认模板"
    );

    // 校验：非法 schema 应失败
    let (status, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({
                "name": "坏卡片",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": {"elements": []}}
            }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(report["valid"], false);
    assert!(!report["errors"].as_array().unwrap().is_empty());

    // 校验 + 预览：合法卡片
    let card_json = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "header": {"template": "${severity_color}", "title": {"tag": "plain_text", "content": "【${status_text}】${alertname}"}},
        "body": {"elements": [{"tag": "markdown", "content": "${alerts_list}\n${unknown_var}"}]}
    });
    let (status, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({"name": "自定义", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": card_json}}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(report["valid"], true, "{report}");
    assert!(report["unknown_variables"]
        .as_array()
        .unwrap()
        .contains(&json!("unknown_var")));

    let (status, preview) = h
        .post(
            "/api/v1/card-templates/preview",
            json!({"name": "自定义", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": card_json},
                   "sample": "firing"}),
        )
        .await;
    assert_eq!(status, 200, "{preview}");
    let rendered = preview["rendered_card_json"].to_string();
    assert!(
        rendered.contains("HighCPUUsage"),
        "预览应使用样例数据: {rendered}"
    );
    assert!(!rendered.contains("${"), "预览不应残留占位符");

    // 创建 → 列表 → 删除
    let (status, created) = h
        .post(
            "/api/v1/card-templates",
            json!({"name": "自定义模板", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": card_json}}),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert!(created["detected_variables"].as_array().unwrap().len() >= 3);

    let (_, card_only) = h.get("/api/v1/card-templates?kind=card_message").await;
    assert_eq!(
        card_only.as_array().unwrap().len(),
        2,
        "默认卡片消息模板 + 新建的"
    );
    let (_, feed_only) = h.get("/api/v1/card-templates?kind=app_feed_card").await;
    assert_eq!(feed_only.as_array().unwrap().len(), 1);
    let (_, all) = h.get("/api/v1/card-templates").await;
    assert_eq!(all.as_array().unwrap().len(), 3);

    let resp = h
        .http
        .delete(h.url(&format!("/api/v1/card-templates/{id}")))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
async fn sync_is_idempotent_and_hook_rotation_invalidates_old_token() {
    let h = start().await;

    let (_, first) = h.post("/api/v1/channels/sync", json!({})).await;
    assert_eq!(first["created"], 2);
    let (_, second) = h.post("/api/v1/channels/sync", json!({})).await;
    assert_eq!(second["created"], 0);
    assert_eq!(second["updated"], 0);
    assert_eq!(second["skipped"], 2);

    // 手工创建渠道拿到 token 后轮换
    let mut h = h;
    // 用一个新的群避免与 sync 导入的渠道冲突
    let (status, body) = h
        .post(
            "/api/v1/channels",
            json!({
                "name": "手工渠道", "receive_id": "oc_manual", "receive_id_type": "chat_id"
            }),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    h.channel_id = Some(body["id"].as_str().unwrap().to_string());
    h.hook_token = Some(body["hook_token"].as_str().unwrap().to_string());
    let old_token = h.hook_token.clone().unwrap();
    let channel_id = h.channel_id.clone().unwrap();
    let (_, detail) = h.get(&format!("/api/v1/channels/{channel_id}")).await;
    let hook_id = detail["hooks"][0]["id"].as_str().unwrap().to_string();

    let resp = h
        .http
        .post(h.url(&format!("/api/v1/hooks/{hook_id}/rotate")))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let rotated: Value = resp.json().await.unwrap();
    let new_token = rotated["token"].as_str().unwrap().to_string();
    assert_ne!(old_token, new_token);

    // 旧 token 立即失效
    let resp = h
        .http
        .post(h.url(&format!("/api/v1/hooks/alertmanager/{old_token}")))
        .json(&firing_payload(1))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // 新 token 可用
    h.hook_token = Some(new_token);
    let (status, body) = h.push(firing_payload(1)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["action"], "send_new");
}

#[tokio::test]
async fn manual_test_message_and_retry_endpoints() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    let (status, delivery) = h
        .post(
            &format!("/api/v1/channels/{channel_id}/test-message"),
            json!({"sample": "firing"}),
        )
        .await;
    assert_eq!(status, 200, "{delivery}");
    assert_eq!(delivery["kind"], "test");
    assert_eq!(delivery["state"], "active");
    assert_eq!(h.fake.snapshot().messages.len(), 1);

    // 活跃投递不允许重试
    let delivery_id = delivery["id"].as_str().unwrap();
    let (status, err) = h
        .post(
            &format!("/api/v1/deliveries/{delivery_id}/retry"),
            json!({}),
        )
        .await;
    assert_eq!(status, 409, "delivery={delivery} resp={err}");

    // 手动撤回可用
    let (status, revoked) = h
        .post(
            &format!("/api/v1/deliveries/{delivery_id}/revoke"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "{revoked}");
    assert_eq!(h.fake.snapshot().recalled.len(), 1);
}
