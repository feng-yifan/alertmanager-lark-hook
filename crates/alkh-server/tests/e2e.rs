//! 端到端集成测试：真实 Axum HTTP 栈 + 内存 SQLite + Fake 飞书网关。
//!
//! 覆盖：登录鉴权 → 渠道创建 → Hook 签发 → AlertManager 投递 →
//! 新告警 / 幂等去重 / 分组更新（原地更新）/ 解决告警 → 查询与指标。

use std::sync::{Arc, Mutex};

use alkh_domain::channel::ReceiveIdType;
use alkh_domain::error::{LarkError, PortError};
use alkh_domain::ports::{LarkChat, LarkGateway, LarkSendOutcome};
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
    updated_card_entities: Vec<(String, Value, i64)>,
    updated_messages: Vec<(String, Value)>,
    recalled: Vec<String>,
    pinned: Vec<String>,
    unpinned: Vec<String>,
    seq: u64,
    /// 注入故障：接下来 `fail_send_times` 次「发送」直接返回 `fail_send_code`。
    /// 用于覆盖发送失败路径（默认 0 = 不注入）。
    fail_send_times: usize,
    fail_send_code: i64,
    /// 注入错误的 `retryable`。真实网关按 **HTTP 状态**判定（429/5xx 才可重试），
    /// 业务错误码数值大小与「能否重试」无关，因此必须显式给出。
    fail_send_retryable: bool,
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
        let retryable = code == LarkError::CODE_RATE_LIMITED || code >= 500;
        self.inject_send_failure_with(times, code, retryable);
    }

    /// 注入一个**不可重试**的发送失败（模拟权限不足 99991672、参数错误等 HTTP 400）。
    ///
    /// 这类错误会立刻把投递置为 `failed`，且后台重试任务只处理 `pending`，
    /// 因此「重试」只能靠用户手动触发。
    fn inject_permanent_send_failure(&self, times: usize, code: i64) {
        self.inject_send_failure_with(times, code, false);
    }

    fn inject_send_failure_with(&self, times: usize, code: i64, retryable: bool) {
        let mut s = self.snapshot();
        s.fail_send_times = times;
        s.fail_send_code = code;
        s.fail_send_retryable = retryable;
    }

    fn take_injected_send_failure(&self) -> Option<PortError> {
        let mut s = self.snapshot();
        if s.fail_send_times == 0 {
            return None;
        }
        s.fail_send_times -= 1;
        let code = s.fail_send_code;
        Some(PortError::Lark(
            LarkError::new(code, "注入的发送失败（测试用）").retryable(s.fail_send_retryable),
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

    async fn pin_message(&self, message_id: &str) -> alkh_domain::PortResult<()> {
        self.snapshot().pinned.push(message_id.to_string());
        Ok(())
    }

    async fn unpin_message(&self, message_id: &str) -> alkh_domain::PortResult<()> {
        self.snapshot().unpinned.push(message_id.to_string());
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
fn test_config(hook_rate_limit: u32) -> Config {
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
        hook_rate_limit,
        retry_interval_secs: 3600,
        retention_interval_secs: 3600,
    }
}

/// 默认脚手架：只起 HTTP 服务，**不启动后台 worker**，避免其它测试被后台重试干扰。
async fn start() -> Harness {
    // 限流额度调高，避免干扰其它用例（限流本身有专门的用例覆盖）
    start_opts(false, 100_000).await
}

/// 需要覆盖聚合冲刷 / 重试 / 保留清理等**后台循环**时使用。
async fn start_with_workers() -> Harness {
    start_opts(true, 100_000).await
}

/// 用较低的 hook 限流额度启动，用于覆盖未认证入口的限流。
async fn start_with_hook_limit(limit: u32) -> Harness {
    start_opts(false, limit).await
}

async fn start_opts(with_workers: bool, hook_rate_limit: u32) -> Harness {
    let config = Arc::new(test_config(hook_rate_limit));
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

    if with_workers {
        alkh_server::workers::spawn(state.clone());
    }

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

    async fn patch(&self, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        let resp = self
            .http
            .patch(self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .expect("PATCH 失败");
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

/// 从 `/metrics` 读取一个计数器的值（Prometheus 文本格式）。
async fn metric(h: &Harness, name: &str) -> u64 {
    let text = h
        .http
        .get(h.url("/metrics"))
        .send()
        .await
        .expect("/metrics 请求失败")
        .text()
        .await
        .expect("/metrics 读取失败");
    text.lines()
        .find(|l| l.starts_with(&format!("{name} ")))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// 轮询等待某个计数器达到条件。
async fn wait_for_metric(h: &Harness, name: &str, at_least: u64, secs: u64) -> u64 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut last = 0;
    while std::time::Instant::now() < deadline {
        last = metric(h, name).await;
        if last >= at_least {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    last
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

    const ENUM_FIELDS: [&str; 7] = [
        "repeat_policies",
        "resolved_policies",
        "group_update_policies",
        "match_ops",
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

    // 组件目录（本地卡片构建工具的契约来源）：驱动面板 + 校验。
    let components = meta["card_components"]
        .as_array()
        .expect("card_components 必须是数组");
    let tags: Vec<&str> = components
        .iter()
        .map(|c| c["tag"].as_str().unwrap())
        .collect();
    for expected in ["markdown", "div", "hr", "button", "column_set", "column"] {
        assert!(
            tags.contains(&expected),
            "组件目录缺少 {expected}: {tags:?}"
        );
    }
    // 已废弃组件必须在目录里（校验时要能给出可操作的报错）
    let note = components
        .iter()
        .find(|c| c["tag"] == "note")
        .expect("目录应包含已废弃的 note");
    assert!(note["deprecated"].as_str().unwrap().contains("notation"));
    assert_eq!(note["palette"], false);
    // 默认 JSON 必须是合法结构（前端新建节点直接用它）
    for component in components {
        if component["palette"] == true {
            let node: Value = serde_json::from_str(component["default_json"].as_str().unwrap())
                .expect("default_json 必须是合法 JSON");
            assert_eq!(node["tag"], component["tag"]);
        }
        assert!(component["props"].is_array());
    }
    // `type` 字段名不得泄露 Rust 字段名 `ty`
    let prop = &components.iter().find(|c| c["tag"] == "markdown").unwrap()["props"][0];
    assert!(prop["type"].is_string(), "{prop}");
    assert!(prop.get("ty").is_none(), "{prop}");

    // 告警积木同样由服务端下发（保证只用真实变量）
    let presets = meta["card_presets"]
        .as_array()
        .expect("card_presets 必须是数组");
    assert!(presets.len() >= 5, "积木太少：{}", presets.len());
    for preset in presets {
        let nodes: Value =
            serde_json::from_str(preset["nodes_json"].as_str().unwrap()).expect("nodes_json 合法");
        assert!(nodes.as_array().is_some_and(|a| !a.is_empty()), "{preset}");
        assert!(preset["label"].is_string(), "{preset}");
    }
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
    assert!(meta["variables"].as_array().unwrap().len() > 20);
    assert!(meta["card_components"].as_array().unwrap().len() >= 5);

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
async fn settings_timezone_round_trips_and_rejects_invalid_names() {
    let h = start().await;

    // 未设置过 → 默认 UTC（保持既有卡片时间行为）
    let (status, settings) = h.get("/api/v1/settings").await;
    assert_eq!(status, 200);
    assert_eq!(settings["timezone"], "UTC");

    // 设为东八区
    let resp = h
        .http
        .put(h.url("/api/v1/settings"))
        .bearer_auth(&h.token)
        .json(&json!({
            "lark_base_url": "https://open.feishu.cn",
            "timezone": "Asia/Shanghai",
            "retention": {"alert_group_days": 30, "delivery_days": 30, "audit_days": 14}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["timezone"], "Asia/Shanghai");

    let (status, settings) = h.get("/api/v1/settings").await;
    assert_eq!(status, 200);
    assert_eq!(settings["timezone"], "Asia/Shanghai");

    // 非法 IANA 名被拒（不落库）
    let resp = h
        .http
        .put(h.url("/api/v1/settings"))
        .bearer_auth(&h.token)
        .json(&json!({
            "lark_base_url": "https://open.feishu.cn",
            "timezone": "Asia/Shangai",
            "retention": {"alert_group_days": 30, "delivery_days": 30, "audit_days": 14}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let (status, settings) = h.get("/api/v1/settings").await;
    assert_eq!(status, 200);
    assert_eq!(settings["timezone"], "Asia/Shanghai");
}

#[tokio::test]
async fn channel_sync_imports_feishu_chats_and_exposes_hook_urls() {
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
    assert_eq!(normal["policy"]["repeat_policy"], "suppress");
    assert_eq!(normal["policy"]["resolved_policy"], "send");

    // Hook 地址不再「只显示一次」：详情接口直接回显完整地址，且主机来自 public_base_url
    // （测试里是 http://alkh.test），不是本次请求的 Host/浏览器地址。
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
        assert_eq!(
            hook["hook_url_available"], true,
            "同步创建的 Hook 也必须能回显地址: {hook}"
        );
        let url = hook["hook_url"].as_str().unwrap();
        assert!(
            url.starts_with("http://alkh.test/api/v1/hooks/alertmanager/alkh_"),
            "地址必须由 public_base_url 拼出，且含完整 token: {url}"
        );
        assert!(!url.contains('…'), "地址不应再被遮蔽: {url}");
        assert_eq!(
            hook["hook_path"].as_str().unwrap(),
            url.trim_start_matches("http://alkh.test"),
            "hook_path 与 hook_url 必须指向同一个 token"
        );
        assert!(
            hook["hook_url"]
                .as_str()
                .unwrap()
                .contains(hook["token_prefix"].as_str().unwrap()),
            "URL 里的 token 应与 token_prefix 对应: {hook}"
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

/// 读取某个分组当前的 Pin 消息列表（R4）。
async fn group_pinned_messages(h: &Harness) -> Vec<String> {
    let raw: String = sqlx::query_scalar("SELECT pinned_messages FROM alert_groups LIMIT 1")
        .fetch_one(&h.pool)
        .await
        .expect("读取 alert_groups 失败");
    let items: Vec<Value> =
        serde_json::from_str(&raw).expect("pinned_messages 必须是合法 JSON 数组");
    items
        .into_iter()
        .filter_map(|v| v["message_id"].as_str().map(str::to_string))
        .collect()
}

/// 把渠道策略里的某个开关改成指定值并保存。
async fn put_policy_flag(h: &Harness, channel_id: &str, key: &str, value: Value) {
    let (status, mut policy) = h
        .get(&format!("/api/v1/channels/{channel_id}/policy"))
        .await;
    assert_eq!(status, 200, "读取策略失败: {policy}");
    policy[key] = value;
    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&policy)
        .send()
        .await
        .expect("保存策略失败");
    assert_eq!(resp.status(), 200, "保存策略应成功");
}

/// R4：开启「未解决告警 Pin」后，firing 期间卡片被 Pin，全部恢复后取消 Pin。
#[tokio::test]
async fn pin_unresolved_pins_firing_message_and_unpins_on_resolve() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    // 默认关闭：firing 也不 Pin（opt-in，避免存量渠道行为突变）。
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["action"], "send_new");
    assert!(
        h.fake.snapshot().pinned.is_empty(),
        "开关默认关闭时不应调用 Pin 接口"
    );

    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;

    // 分组内容变化 → 原地更新同一条卡片；此时才第一次 Pin。
    let (_, body) = h.push(firing_payload(2)).await;
    assert_eq!(body["event"], "updated");
    let pinned = {
        let s = h.fake.snapshot();
        assert_eq!(s.pinned.len(), 1, "未解决时应 Pin 当前活跃卡片");
        s.pinned[0].clone()
    };
    assert_eq!(
        group_pinned_messages(&h).await,
        vec![pinned.clone()],
        "Pin 消息必须落库，恢复时才能精确取消"
    );
    assert!(
        metric(&h, "alkh_messages_pinned_total").await >= 1,
        "Pin 成功应计入指标"
    );
    // HTTP 线格式也要暴露 Pin 状态，便于后台排查「到底有没有 Pin 成功」。
    let group_id = body["alert_group_id"]
        .as_str()
        .expect("缺少 alert_group_id");
    let (status, group) = h.get(&format!("/api/v1/alert-groups/{group_id}")).await;
    assert_eq!(status, 200);
    let pinned_list = group["pinned_messages"]
        .as_array()
        .expect("pinned_messages 应为数组");
    assert!(
        pinned_list
            .iter()
            .any(|p| p["message_id"].as_str() == Some(pinned.as_str())),
        "HTTP 响应应暴露当前 Pin 的消息"
    );

    // 再次内容变化：卡片被原地更新，仍是同一条消息 ⇒ 不应重复调用 Pin 接口。
    let (_, body) = h.push(firing_payload(3)).await;
    assert_eq!(body["event"], "updated");
    assert_eq!(
        h.fake.snapshot().pinned.len(),
        1,
        "同一条消息不应重复 Pin（Pin 接口幂等，但没必要每次都调）"
    );

    // 全部恢复 → 发恢复卡片 + 取消旧卡片的 Pin。
    let (_, body) = h.push(resolved_payload()).await;
    assert_eq!(body["event"], "resolved");
    {
        let s = h.fake.snapshot();
        assert_eq!(s.unpinned, vec![pinned.clone()], "恢复后必须取消 Pin");
    }
    assert!(
        group_pinned_messages(&h).await.is_empty(),
        "取消 Pin 后分组记录必须清空"
    );
    assert!(
        metric(&h, "alkh_messages_unpinned_total").await >= 1,
        "取消 Pin 成功应计入指标"
    );
}

/// R4：firing 期间关闭开关，下一个事件就要把旧 Pin 取消掉（不能永久留一条 Pin）。
#[tokio::test]
async fn disabling_pin_unresolved_unpins_on_next_event() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;

    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["action"], "send_new");
    let pinned = {
        let s = h.fake.snapshot();
        assert_eq!(s.pinned.len(), 1, "开启后 firing 应 Pin");
        s.pinned[0].clone()
    };

    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(false)).await;
    let (_, body) = h.push(firing_payload(2)).await;
    assert_eq!(body["event"], "updated");
    {
        let s = h.fake.snapshot();
        assert_eq!(s.unpinned, vec![pinned], "关闭开关后下一个事件应取消 Pin");
    }
    assert!(group_pinned_messages(&h).await.is_empty());
}

/// R4（多条 Pin）：内容变化策略为「发送」时，每张新卡片都会累积一条 Pin；
/// 全部恢复且「自动取消 Pin」开启（默认）时，批量取消所有未解决 Pin。
#[tokio::test]
async fn pin_unresolved_accumulates_and_unpins_all_on_resolve() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;
    // 内容变化时发送新消息，制造多张卡片。
    put_policy_flag(&h, &channel_id, "group_update_policy", json!("send")).await;

    h.push(firing_payload(1)).await;
    h.push(firing_payload(2)).await;
    h.push(firing_payload(3)).await;

    let pinned = h.fake.snapshot().pinned.clone();
    assert_eq!(pinned.len(), 3, "每次新卡片都应累积一条 Pin");
    assert_eq!(
        group_pinned_messages(&h).await.len(),
        3,
        "三条 Pin 都必须落库"
    );

    h.push(resolved_payload()).await;
    {
        let s = h.fake.snapshot();
        for id in &pinned {
            assert!(
                s.unpinned.contains(id),
                "恢复后应批量取消所有未解决 Pin: {id}"
            );
        }
    }
    assert!(
        group_pinned_messages(&h).await.is_empty(),
        "批量取消后 Pin 记录应清空"
    );
}

/// R4（多条 Pin）：关闭「全部恢复后自动取消 Pin」后，恢复时保留未解决 Pin。
#[tokio::test]
async fn disabling_unpin_on_resolve_keeps_pins() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;
    put_policy_flag(&h, &channel_id, "unpin_on_resolve", json!(false)).await;
    put_policy_flag(&h, &channel_id, "group_update_policy", json!("send")).await;

    h.push(firing_payload(1)).await;
    h.push(firing_payload(2)).await;
    let pinned = h.fake.snapshot().pinned.clone();
    assert_eq!(pinned.len(), 2);

    h.push(resolved_payload()).await;
    {
        let s = h.fake.snapshot();
        assert!(
            s.unpinned.is_empty(),
            "关闭自动取消后恢复时不应取消未解决 Pin"
        );
    }
    assert_eq!(
        group_pinned_messages(&h).await,
        pinned,
        "未解决 Pin 应全部保留"
    );
}

/// R4b：开启「已解决告警 Pin」后，恢复时把恢复卡片也 Pin 上（与未解决 Pin 独立）。
#[tokio::test]
async fn pin_resolved_pins_the_resolution_card() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    // 只开「已解决 Pin」，不开「未解决 Pin」。
    put_policy_flag(&h, &channel_id, "pin_resolved", json!(true)).await;

    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["action"], "send_new");
    assert!(
        h.fake.snapshot().pinned.is_empty(),
        "未开启未解决 Pin 时 firing 不应 Pin"
    );

    h.push(resolved_payload()).await;
    assert_eq!(h.fake.snapshot().pinned.len(), 1, "恢复时应 Pin 恢复卡片");
    let raw: String = sqlx::query_scalar("SELECT pinned_messages FROM alert_groups LIMIT 1")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    let items: Vec<Value> = serde_json::from_str(&raw).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "resolved", "记录角色应为 resolved");
}

/// R4/R4b：两组同时开启且关闭自动取消时，未解决 Pin 保留、恢复卡片也额外 Pin（多条并存）。
#[tokio::test]
async fn unresolved_and_resolved_pins_can_coexist() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;
    put_policy_flag(&h, &channel_id, "unpin_on_resolve", json!(false)).await;
    put_policy_flag(&h, &channel_id, "pin_resolved", json!(true)).await;
    put_policy_flag(&h, &channel_id, "group_update_policy", json!("send")).await;

    h.push(firing_payload(1)).await;
    h.push(firing_payload(2)).await;
    let firing_pins = h.fake.snapshot().pinned.clone();
    assert_eq!(firing_pins.len(), 2);

    h.push(resolved_payload()).await;
    {
        let s = h.fake.snapshot();
        assert_eq!(s.pinned.len(), 3, "两张未解决 + 一张恢复卡片共三条 Pin");
        assert!(s.unpinned.is_empty(), "关闭自动取消时不应取消未解决 Pin");
    }
    assert_eq!(group_pinned_messages(&h).await.len(), 3);
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
    let template_id: String =
        sqlx::query_scalar("SELECT id FROM card_templates WHERE is_default = 1")
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

/// 回归：未认证的 hook 接收端点必须有来源级限流。
///
/// 该端点不需要认证（AlertManager 只带 token），任何来源都能打，每次请求都会产生
/// DB 查询、token 有效时还会触发飞书调用。此前完全没有限流：单个来源就能把实例打满。
#[tokio::test]
async fn hook_endpoint_is_rate_limited_per_source() {
    let mut h = start_with_hook_limit(3).await;
    let _ = h.create_channel().await;

    // 额度内放行（第 2、3 次会因幂等被判 duplicate，但仍是 200）
    for i in 0..3 {
        let (status, body) = h.push(firing_payload(1)).await;
        assert_eq!(status, 200, "第 {i} 次应放行: {body}");
    }

    // 超限被拒
    let (status, body) = h.push(firing_payload(1)).await;
    assert_eq!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "超过额度应被限流: {body}"
    );
    assert_eq!(body["error"]["code"], "rate_limited");
    assert!(
        body["error"]["details"]["retry_after_secs"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );

    // 合法调用方只要来自别的来源就不受牵连（否则会连带丢掉真告警）
    let token = h.hook_token.clone().unwrap();
    let status = h
        .http
        .post(h.url(&format!("/api/v1/hooks/alertmanager/{token}")))
        .header("x-forwarded-for", "203.0.113.9")
        .json(&firing_payload(1))
        .send()
        .await
        .expect("webhook 请求失败")
        .status();
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "其它来源不应被牵连（会把真告警一起拒掉）"
    );

    // token 无效同样受限流保护（连 DB 查询都不做就被挡住）
    let mut rejected = 0;
    for _ in 0..5 {
        let status = h
            .http
            .post(h.url("/api/v1/hooks/alertmanager/alkh_not_a_real_token"))
            .json(&firing_payload(1))
            .send()
            .await
            .expect("webhook 请求失败")
            .status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            rejected += 1;
        }
    }
    assert!(rejected > 0, "无效 token 的洪水也应被限流挡住");
}

/// 回归：三个「声明了、导出了、却永远为 0」的计数器必须真的有自增点。
///
/// 此前 `deliveries_recalled_total` / `lark_api_errors_total` / `render_failures_total`
/// 只在 `metrics.rs` 里声明并渲染，全仓库没有任何 `inc` —— 运维看到的是恒为 0 的
/// 假指标，比没有指标更糟（会误判「一切正常」）。
#[tokio::test]
async fn previously_dead_counters_are_actually_incremented() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    // ---- 1) 渲染失败 → render_failures_total ----
    // 卡片内容序列化在 card_message 里，其中 card_json 换成非对象
    // （绕过保存时的校验，直接用 SQL 造脏数据）
    let original: String =
        sqlx::query_scalar("SELECT card_message FROM card_templates WHERE is_default = 1")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    sqlx::query(
        "UPDATE card_templates SET card_message = json_set(card_message, '$.card_json', json('[1,2]')) \
         WHERE is_default = 1",
    )
    .execute(&h.pool)
    .await
    .unwrap();
    let (status, body) = h.push(firing_payload(1)).await;
    assert!(
        status.is_client_error() || status.is_server_error(),
        "渲染失败应报错: {status} {body}"
    );
    assert!(
        metric(&h, "alkh_render_failures_total").await >= 1,
        "渲染失败必须被计数"
    );
    sqlx::query("UPDATE card_templates SET card_message = ? WHERE is_default = 1")
        .bind(&original)
        .execute(&h.pool)
        .await
        .unwrap();

    // ---- 2) 飞书报错 → lark_api_errors_total ----
    h.fake.inject_send_failure(1, LarkError::CODE_RATE_LIMITED);
    let mut payload = firing_payload(1);
    payload["alerts"][0]["fingerprint"] = json!("counter-fp-1");
    let (_, body) = h.push(payload).await;
    assert_eq!(body["delivery_state"], "pending", "{body}");
    assert!(
        metric(&h, "alkh_lark_api_errors_total").await >= 1,
        "飞书返回错误必须被计数"
    );

    // ---- 3) 撤回成功 → deliveries_recalled_total ----
    // 先造一条真正发出去的投递（飞书已恢复正常）
    let mut payload = firing_payload(1);
    payload["alerts"][0]["fingerprint"] = json!("counter-fp-2");
    let (_, body) = h.push(payload).await;
    assert_eq!(body["delivery_state"], "active", "{body}");
    let delivery_id = body["delivery_id"].as_str().unwrap().to_string();

    let (status, body) = h
        .post(
            &format!("/api/v1/deliveries/{delivery_id}/revoke"),
            json!({}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        metric(&h, "alkh_deliveries_recalled_total").await >= 1,
        "成功撤回必须被计数"
    );
}

/// 数据丢失回归：渲染后超过飞书 30KB 上限的卡片必须被截断，而不是丢掉整条告警。
///
/// 模板保存时只校验**骨架**体积，真正的告警数据是渲染时注入的：一条超长 annotation
/// 就能把卡片顶过上限，而飞书对超限返回不可重试的错误 ⇒ 该告警永远不会送达。
#[tokio::test]
async fn oversized_rendered_card_is_shrunk_instead_of_dropping_the_alert() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    // 4 字符 × 3 字节 × 6000 ≈ 72KB，远超 30KB
    let huge = "告警描述".repeat(6_000);
    let mut payload = firing_payload(1);
    payload["alerts"][0]["annotations"] = json!({ "summary": huge });
    payload["commonAnnotations"] = json!({ "summary": huge });

    let (status, body) = h.push(payload).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["delivery_state"], "active",
        "超长卡片应被截断后照常发出，而不是丢掉整条告警: {body}"
    );

    let entities = h.fake.snapshot().card_entities.clone();
    assert!(!entities.is_empty(), "应已创建卡片实体");
    for card in &entities {
        let bytes = serde_json::to_vec(card).unwrap().len();
        assert!(
            bytes <= 30 * 1024,
            "发给飞书的卡片仍是 {bytes} 字节，超过 30KB 上限（线上会被拒绝）"
        );
    }
}

/// 数据丢失回归：同一分组的并发事件不得互相覆盖。
///
/// `process_event` 是「读取分组 → 推进状态 → 落库 → 发消息」的读改写，此前没有任何
/// 互斥：两个并发事件各自读到同一份快照，后写者覆盖前者，前者的 `event_count` /
/// `resolved_count` / `last_seen_at` 推进静默丢失，甚至两边都判「需要发新消息」
/// 而产生重复卡片。
#[tokio::test]
async fn concurrent_events_for_same_group_do_not_lose_updates() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    const N: usize = 8;
    let token = h.hook_token.clone().expect("尚未创建 hook");
    let url = h.url(&format!("/api/v1/hooks/alertmanager/{token}"));
    let client = h.http.clone();

    // N 条内容各不相同的事件（幂等键不同、不会被去重），全部落在同一个分组
    let mut tasks = Vec::new();
    for i in 0..N {
        // ⚠️ 幂等键 `event_hash` 只由 fingerprint / status / startsAt 组成，
        // 改 summary 不会改变它 —— 那样 7 条会被正确判为 duplicate。必须换 fingerprint。
        let mut payload = firing_payload(1);
        payload["alerts"][0]["fingerprint"] = json!(format!("concurrent-{i}"));
        payload["alerts"][0]["annotations"] = json!({"summary": format!("并发事件 {i}")});
        let client = client.clone();
        let url = url.clone();
        tasks.push(tokio::spawn(async move {
            client
                .post(&url)
                .json(&payload)
                .send()
                .await
                .expect("webhook 请求失败")
                .status()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), reqwest::StatusCode::OK);
    }

    let (_, groups) = h.get("/api/v1/alert-groups?limit=5").await;
    let items = groups["items"].as_array().expect("应返回分组列表");
    assert_eq!(items.len(), 1, "N 个事件应聚成同一个分组: {groups}");
    assert_eq!(
        items[0]["event_count"].as_u64().unwrap_or(0),
        N as u64,
        "并发事件发生丢更新（读改写未串行），只有部分被计入: {}",
        items[0]
    );
}

/// 数据丢失回归：聚合桶冲刷失败必须放回队列重试，而不是静默丢弃。
///
/// 聚合桶只在进程内存里，AM 又已经收到 200（不会再重发），因此冲刷失败时丢掉桶
/// 等于永久丢失这条告警。原实现直接 `return false`，只在日志里留一行 warn。
#[tokio::test]
async fn failed_aggregate_flush_is_requeued_and_eventually_delivered() {
    let mut h = start_with_workers().await;
    let _ = h.create_channel().await;

    // 开启聚合窗口，让事件先进桶
    let channel_id = h.channel_id.clone().unwrap();
    let resp = h
        .http
        .put(h.url(&format!("/api/v1/channels/{channel_id}/policy")))
        .bearer_auth(&h.token)
        .json(&json!({
            "repeat_policy": "suppress",
            "resolved_policy": "send",
            "group_update_policy": "update_previous",
            "label_matchers": [],
            "aggregation_window_secs": 2,
            "min_send_interval_secs": 0,
            "max_alerts_per_message": 20,
            "mute_until": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);

    // 让冲刷必定失败：把默认卡片模板的内容清空（prepare 会直接报 internal）
    let original: Option<String> =
        sqlx::query_scalar("SELECT card_message FROM card_templates WHERE is_default = 1")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert!(original.is_some(), "应存在默认卡片模板");
    sqlx::query("UPDATE card_templates SET card_message = NULL WHERE is_default = 1")
        .execute(&h.pool)
        .await
        .unwrap();

    // 事件先进聚合桶（响应为 deferred）
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["deferred"], true, "应被聚合窗口延后: {body}");

    // 到期后冲刷失败 ⇒ 必须放回队列（而不是丢弃）
    let requeued = wait_for_metric(&h, "alkh_aggregator_requeued_total", 1, 15).await;
    assert!(
        requeued >= 1,
        "冲刷失败后应放回队列重试，而不是静默丢弃（requeued={requeued}）"
    );
    assert_eq!(
        metric(&h, "alkh_aggregator_dropped_total").await,
        0,
        "可恢复的失败不应被计入丢弃"
    );

    // 修复模板 ⇒ 退避重试应当把这条事件补投出去
    sqlx::query("UPDATE card_templates SET card_message = ? WHERE is_default = 1")
        .bind(original.unwrap())
        .execute(&h.pool)
        .await
        .unwrap();

    let flushed = wait_for_metric(&h, "alkh_aggregator_flushed_total", 1, 25).await;
    assert!(
        flushed >= 1,
        "模板恢复后重试应成功补投（flushed={flushed}）"
    );

    let (_, deliveries) = h.get("/api/v1/deliveries?limit=10").await;
    let states: Vec<&str> = deliveries["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["state"].as_str())
        .collect();
    assert!(
        states.contains(&"active"),
        "最终应有一条投递成功，实际: {states:?}"
    );
}

/// 安全回归：初始密码必须在**服务端**被强制改掉。
///
/// 初始密码会打印到启动日志（且可能被日志采集），此前「必须改密」只是前端提示，
/// 用初始口令即可直接调用全部管理接口 —— 等于该提示形同虚设。
#[tokio::test]
async fn initial_password_must_be_changed_before_other_apis() {
    let h = start().await;
    // 模拟「用随机初始密码引导」后的状态（start() 用的是显式密码，故这里直接置位）
    sqlx::query("UPDATE admin_users SET must_change_password = 1")
        .execute(&h.pool)
        .await
        .unwrap();

    // 其它管理接口被拒
    let (status, body) = h.get("/api/v1/settings").await;
    assert_eq!(
        status,
        reqwest::StatusCode::FORBIDDEN,
        "初始密码未改前不应能调用管理接口: {body}"
    );
    assert_eq!(body["error"]["code"], "forbidden");
    // 写操作同样被拒
    let (status, _) = h
        .post("/api/v1/channels/sync", json!({"create_missing": false}))
        .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);

    // 但会话查询/登出/改密本身必须可用，否则永远出不来
    let (status, _) = h.get("/api/v1/auth/session").await;
    assert_eq!(status, reqwest::StatusCode::OK);

    let (status, body) = h
        .post(
            "/api/v1/auth/password",
            json!({
                "current_password": "test-password-123",
                "new_password": "a-much-better-password"
            }),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "改密本身必须可用: {body}");

    // 置位被清除（改密会销毁全部会话，所以直接查库确认）
    let left: i64 = sqlx::query_scalar("SELECT must_change_password FROM admin_users")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "改密后应清除 must_change_password");
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

/// 回归：手动重试是**当场重发一次**，不是「加入后台队列」。
///
/// 飞书返回**不可重试**的错误（如权限不足 99991672 / HTTP 400）时，投递会直接落成
/// `failed`；此后 `list_retryable` 只挑 `state = pending`，所以这条记录**永远不会**
/// 被后台任务重试。手动重试接口本身仍返回 200 + `state = failed`，前端若无视返回的
/// `state` 就会提示「已提交重试，等待后台队列重新发送」——用户点击后什么都不会发生。
#[tokio::test]
async fn manual_retry_of_permanent_failure_returns_200_but_stays_failed() {
    let mut h = start().await;
    let _ = h.create_channel().await;

    // 99991672：缺少 cardkit 权限，HTTP 400 ⇒ 不可重试 ⇒ 直接 failed（不进后台队列）。
    h.fake.inject_permanent_send_failure(usize::MAX, 99991672);
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(
        body["delivery_state"], "failed",
        "不可重试的错误应直接落 failed，而不是 pending: {body}"
    );
    let id = body["delivery_id"]
        .as_str()
        .expect("应有投递 ID")
        .to_string();

    // 第一次手动重试：仍然失败，但 HTTP 是 200（前端因此误报成功）。
    let (status, retried) = h
        .post(
            &format!("/api/v1/deliveries/{id}/retry"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, 200, "接口返回 200，但不代表消息已发出: {retried}");
    assert_eq!(retried["state"], "failed", "权限未修复前重试必然再失败");
    assert!(
        retried["last_error"]
            .as_str()
            .is_some_and(|e| e.contains("99991672")),
        "失败原因必须回传给前端，否则用户看不到「为什么没收到」: {retried}"
    );

    // 后台重试集合只看 pending ⇒ 这条 failed 永远不会被自动重试。
    let pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM deliveries WHERE state = 'pending'")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(pending, 0, "failed 不在后台重试集合里，不会自动重发");

    // 外部原因修复（补权限）后，手动重试才真正发得出去。
    h.fake.inject_send_failure(0, 0);
    let (status, retried) = h
        .post(
            &format!("/api/v1/deliveries/{id}/retry"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        retried["state"], "active",
        "原因修复后手动重试必须真正发出: {retried}"
    );
    assert_eq!(
        h.fake.snapshot().messages.len(),
        1,
        "重试成功后应真的调用了一次飞书发送"
    );
}

/// 分组被删除后，投递记录还留在库里（`alert_groups` 删除不级联 `deliveries`），
/// 此时点「重试」必须拿到 **404** 而不是假成功：没有分组上下文就重建不出投递。
///
/// 本机 `data/alkh.db` 里就有这样的悬空投递：`failed` 状态、`alert_group_id`
/// 在 `alert_groups` 里已经不存在，重试只会报「告警分组不存在，无法重试」。
#[tokio::test]
async fn retry_of_delivery_with_deleted_group_is_404() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    h.fake.inject_permanent_send_failure(usize::MAX, 99991672);
    let (_, body) = h.push(firing_payload(1)).await;
    let gid = body["alert_group_id"].as_str().unwrap().to_string();
    let id = body["delivery_id"].as_str().unwrap().to_string();

    // 直接删分组行：等价于保留清理/手动删除分组后留下的悬空投递。
    sqlx::query("DELETE FROM alert_groups WHERE id = ?")
        .bind(&gid)
        .execute(&h.pool)
        .await
        .unwrap();

    let (status, err) = h
        .post(
            &format!("/api/v1/deliveries/{id}/retry"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "悬空投递不可重试: {err}"
    );
    assert!(
        err["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("告警分组不存在")),
        "必须给出可读原因: {err}"
    );
    // 一次真实的发送都没发生。
    assert!(h.fake.snapshot().messages.is_empty());
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
        1,
        "应有一个内置默认卡片模板"
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

    let (_, all) = h.get("/api/v1/card-templates").await;
    assert_eq!(all.as_array().unwrap().len(), 2, "默认模板 + 新建的");

    let resp = h
        .http
        .delete(h.url(&format!("/api/v1/card-templates/{id}")))
        .bearer_auth(&h.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

/// 编辑器元数据 `card_message.builder_meta`：随模板落库、原样回传，
/// 但**不进入渲染**，也不会出现在发给飞书的卡片 JSON 里。
#[tokio::test]
async fn card_template_builder_meta_round_trips() {
    let h = start().await;
    let card_json = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "body": {"elements": [{"tag": "markdown", "element_id": "summary", "content": "${alerts_summary}"}]}
    });
    let meta = json!({"labels": {"summary": "总体状态"}});

    // 创建时带上元数据
    let (status, created) = h
        .post(
            "/api/v1/card-templates",
            json!({
                "name": "带元数据的模板",
                "card_message": {
                    "source": "builder_json",
                    "card_json": card_json,
                    "builder_meta": meta
                }
            }),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(
        created["card_message"]["builder_meta"]["labels"]["summary"],
        "总体状态"
    );
    // 元数据不属于卡片 JSON
    assert!(
        !created["card_message"]["card_json"]
            .to_string()
            .contains("builder_meta"),
        "{created}"
    );
    let id = created["id"].as_str().unwrap().to_string();

    // 读回来还在
    let (status, fetched) = h.get(&format!("/api/v1/card-templates/{id}")).await;
    assert_eq!(status, 200, "{fetched}");
    assert_eq!(
        fetched["card_message"]["builder_meta"]["labels"]["summary"],
        "总体状态"
    );

    // 预览不因元数据而改变渲染结果
    let (status, preview) = h
        .post(
            "/api/v1/card-templates/preview",
            json!({
                "name": "带元数据的模板",
                "card_message": {
                    "source": "builder_json",
                    "card_json": card_json,
                    "builder_meta": meta
                }
            }),
        )
        .await;
    assert_eq!(status, 200, "{preview}");
    assert!(
        !preview["rendered_card_json"]
            .to_string()
            .contains("总体状态"),
        "编辑器元数据不得进入渲染: {preview}"
    );

    // PATCH 也可以覆盖 / 清空
    let (status, patched) = h
        .patch(
            &format!("/api/v1/card-templates/{id}"),
            json!({
                "name": "带元数据的模板",
                "card_message": {
                    "source": "builder_json",
                    "card_json": card_json,
                    "builder_meta": {"labels": {"summary": "新名字"}}
                }
            }),
        )
        .await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(
        patched["card_message"]["builder_meta"]["labels"]["summary"],
        "新名字"
    );

    // 老客户端不带该字段：仍可保存，且不会再写出来
    let (status, legacy) = h
        .patch(
            &format!("/api/v1/card-templates/{id}"),
            json!({
                "name": "带元数据的模板",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 200, "{legacy}");
    assert!(
        legacy["card_message"]["builder_meta"].is_null(),
        "不带元数据时应清空而不是保留旧值: {legacy}"
    );
}

/// P1：mock 基线、带覆盖的预览、草稿测试发送。
#[tokio::test]
async fn card_template_mock_baseline_preview_and_draft_send_test() {
    let mut h = start().await;
    h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();

    // --- 1. 内置基线：返回全部变量 ---
    let (status, baseline) = h.get("/api/v1/card-templates/mock?sample=firing").await;
    assert_eq!(status, 200, "{baseline}");
    assert_eq!(baseline["source"], "builtin");
    assert_eq!(baseline["variables"]["alertname"], "HighCPUUsage");
    assert_eq!(baseline["variables"]["status_text"], "未解决");
    assert!(baseline["variables"]["alerts_list"]
        .as_str()
        .unwrap()
        .contains("HighCPUUsage"));

    // --- 2. 覆盖项进入预览渲染 ---
    let card_json = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "header": {"template": "${severity_color}", "title": {"tag": "plain_text", "content": "${alertname}"}},
        "body": {"elements": [
            {"tag": "markdown", "content": "${summary} / ${label:team}"}
        ]}
    });
    let (status, preview) = h
        .post(
            "/api/v1/card-templates/preview",
            json!({
                "sample": "firing",
                "channel_id": channel_id,
                "mock": {
                    "variables": {"summary": "来自 mock 的概要", "label:team": "sre-mock"}
                },
                "name": "mock 预览",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 200, "{preview}");
    let rendered = preview["rendered_card_json"].to_string();
    assert!(rendered.contains("来自 mock 的概要"), "{rendered}");
    assert!(
        rendered.contains("sre-mock"),
        "动态 label 覆盖应生效: {rendered}"
    );
    // 预览返回的是**覆盖后**的有效上下文
    assert_eq!(preview["variables"]["summary"], "来自 mock 的概要");

    // --- 3. 用历史告警分组作为 mock 基线 ---
    let (status, pushed) = h.push(firing_payload(1)).await;
    assert_eq!(status, 200, "{pushed}");
    let (_, groups) = h.get("/api/v1/alert-groups").await;
    let group_id = groups["items"][0]["id"].as_str().unwrap().to_string();

    let (status, from_history) = h
        .get(&format!(
            "/api/v1/card-templates/mock?sample=firing&alert_group_id={group_id}"
        ))
        .await;
    assert_eq!(status, 200, "{from_history}");
    assert_eq!(from_history["source"], "alert_group");
    assert_eq!(from_history["alert_group_id"], group_id);
    assert_eq!(from_history["variables"]["alertname"], "HighCPU");
    assert!(
        from_history["alert_group_label"]
            .as_str()
            .unwrap()
            .contains("HighCPU"),
        "{from_history}"
    );

    // 历史分组不存在 → 404（而不是静默回退到样例）
    let missing = alkh_domain::ids::AlertGroupId::new();
    let (status, body) = h
        .get(&format!(
            "/api/v1/card-templates/mock?alert_group_id={missing}"
        ))
        .await;
    assert_eq!(status, 404, "{body}");

    // --- 4. 草稿测试发送：不落库模板，也不写告警分组 / 投递记录 ---
    let (_, before) = h.get("/api/v1/card-templates").await;
    let templates_before = before.as_array().unwrap().len();
    // 记录发送前的分组 / 投递条数：样式验证消息不应该让它们变化
    let (_, groups_before_body) = h.get("/api/v1/alert-groups").await;
    let groups_before = groups_before_body["total"].as_u64().unwrap_or(0);
    let (_, deliveries_before_body) = h.get("/api/v1/deliveries").await;
    let deliveries_before = deliveries_before_body["total"].as_u64().unwrap_or(0);

    let (status, delivery) = h
        .post(
            "/api/v1/card-templates/send-test",
            json!({
                "channel_id": channel_id,
                "sample": "firing",
                "mock": {"variables": {"summary": "草稿测试概要"}},
                "name": "草稿模板",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 200, "{delivery}");
    assert_eq!(delivery["state"], "active");
    assert_eq!(delivery["kind"], "test");
    // 草稿不落库 ⇒ Delivery 不记录 template_id
    assert!(delivery["card_template_id"].is_null(), "{delivery}");

    {
        let s = h.fake.snapshot();
        // 第 3 步真实推送也创建过卡片实体，这里只关心最近一次（草稿测试）。
        let rendered = s.card_entities.last().expect("应创建卡片实体").to_string();
        assert!(rendered.contains("草稿测试概要"), "{rendered}");
    }

    let (_, after) = h.get("/api/v1/card-templates").await;
    assert_eq!(
        after.as_array().unwrap().len(),
        templates_before,
        "草稿测试不得创建模板"
    );

    // --- 4b. 连续两次样式验证都必须成功，且什么都不记录 ---
    // 曾经每次测试都用新 id 往 alert_groups INSERT，第二次就报
    // `UNIQUE constraint failed: alert_groups.hook_id, alert_groups.group_key`。
    // 现在卡片编辑器的测试发送完全不落库，冲突与污染都不存在。
    let (status, second) = h
        .post(
            "/api/v1/card-templates/send-test",
            json!({
                "channel_id": channel_id,
                "sample": "firing",
                "mock": {"variables": {"summary": "草稿测试概要"}},
                "name": "草稿模板",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 200, "连续第二次测试发送不应报唯一约束：{second}");
    assert_eq!(second["kind"], "test", "{second}");

    let (_, groups_after_body) = h.get("/api/v1/alert-groups").await;
    assert_eq!(
        groups_after_body["total"].as_u64().unwrap_or(0),
        groups_before,
        "样式验证消息不得写入告警分组: {groups_after_body}"
    );
    let (_, deliveries_after_body) = h.get("/api/v1/deliveries").await;
    assert_eq!(
        deliveries_after_body["total"].as_u64().unwrap_or(0),
        deliveries_before,
        "样式验证消息不得写入投递记录: {deliveries_after_body}"
    );

    // --- 4c. 指定历史分组作为 mock 基线时同样不落库 ---
    // 这是最容易漏的路径：mock 指向一条真实分组时，曾经会把样例分组写库。
    let (status, historical) = h
        .post(
            "/api/v1/card-templates/send-test",
            json!({
                "channel_id": channel_id,
                "sample": "firing",
                "mock": {"alert_group_id": group_id},
                "name": "草稿模板",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 200, "历史分组基线也应能发送: {historical}");
    let (_, groups_after_history) = h.get("/api/v1/alert-groups").await;
    assert_eq!(
        groups_after_history["total"].as_u64().unwrap_or(0),
        groups_before,
        "以历史分组为基线也不得写入告警分组: {groups_after_history}"
    );
    let (_, deliveries_after_history) = h.get("/api/v1/deliveries").await;
    assert_eq!(
        deliveries_after_history["total"].as_u64().unwrap_or(0),
        deliveries_before,
        "以历史分组为基线也不得写入投递记录: {deliveries_after_history}"
    );

    // 未选渠道 / 不存在的渠道
    let (status, body) = h
        .post(
            "/api/v1/card-templates/send-test",
            json!({
                "channel_id": "00000000-0000-0000-0000-000000000000",
                "name": "草稿模板",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // --- 5. mock 规模上限 ---
    let too_many: serde_json::Map<String, Value> =
        (0..201).map(|i| (format!("k{i}"), json!("v"))).collect();
    let (status, body) = h
        .post(
            "/api/v1/card-templates/preview",
            json!({
                "sample": "firing",
                "mock": {"variables": too_many},
                "name": "mock 预览",
                "kind": "card_message",
                "card_message": {"source": "builder_json", "card_json": card_json}
            }),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

/// P0：组件目录校验 + 停用 builder_template 来源。
#[tokio::test]
async fn card_catalog_blocks_deprecated_components_and_legacy_source() {
    let h = start().await;

    // 0. 内置默认卡片（编辑器「起步卡片」/旧来源转换的落点）必须自身合法
    let (status, defaults) = h.get("/api/v1/card-templates/default").await;
    assert_eq!(status, 200, "{defaults}");
    assert_eq!(defaults["card_message"]["source"], "builder_json");
    let starter = defaults["card_message"]["card_json"].clone();
    assert_eq!(starter["schema"], "2.0");
    let (status, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({"name": "起步卡片", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": starter}}),
        )
        .await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["valid"], true, "起步卡片必须合法: {report}");
    assert_eq!(
        report["warnings"].as_array().unwrap().len(),
        0,
        "起步卡片不应有警告: {report}"
    );

    // 1. 已废弃的 note（JSON 2.0 不支持）→ 校验失败，且给出可操作的替代写法
    let note_card = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "body": {"elements": [{"tag": "note", "elements": [{"tag": "plain_text", "content": "脚注"}]}]}
    });
    let (status, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({"name": "note 卡", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": note_card}}),
        )
        .await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["valid"], false, "{report}");
    let errors = report["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|e| e.as_str().unwrap().contains("notation")),
        "note 的报错应给出替代写法: {errors:?}"
    );

    // 2. 未知组件是**警告**而不是错误（导入的卡片仍可保存，只是不能可视化编辑）
    let unknown_card = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "body": {"elements": [{"tag": "chart", "chart_spec": {}}]}
    });
    let (_, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({"name": "chart 卡", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": unknown_card}}),
        )
        .await;
    assert_eq!(report["valid"], true, "{report}");
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().contains("chart")));

    // 3. 嵌套容器 + element_id 规则：合法卡片零错误零警告
    let good_card = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "header": {"template": "${severity_color}"},
        "body": {"elements": [
            {"tag": "markdown", "element_id": "summary", "content": "${alerts_summary}"},
            {"tag": "column_set", "flex_mode": "bisect", "columns": [
                {"tag": "column", "width": "weighted", "weight": 1,
                 "elements": [{"tag": "div", "text": {"tag": "plain_text", "content": "${firing_count}"}}]},
                {"tag": "column", "width": "weighted", "weight": 1,
                 "elements": [{"tag": "collapsible_panel", "expanded": false,
                               "header": {"title": {"tag": "plain_text", "content": "详情"}},
                               "elements": [{"tag": "markdown", "content": "${alerts_list}"}]}]}
            ]},
            {"tag": "button", "text": {"tag": "plain_text", "content": "查看详情"},
             "type": "primary", "behaviors": [{"type": "open_url", "default_url": "${external_url}"}]}
        ]}
    });
    let (_, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({"name": "合法卡", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": good_card}}),
        )
        .await;
    assert_eq!(report["valid"], true, "{report}");
    assert_eq!(
        report["warnings"].as_array().unwrap().len(),
        0,
        "合法卡片不应有警告: {report}"
    );

    // 4. 保存同样被拦（不能只在 /validate 里提示）
    let (status, body) = h
        .post(
            "/api/v1/card-templates",
            json!({"name": "note 卡", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": note_card}}),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("结构不合法"),
        "{body}"
    );

    // 5. column 放在 column_set 之外 → 错误
    let orphan_column = json!({
        "schema": "2.0",
        "config": {"update_multi": true},
        "body": {"elements": [{"tag": "column", "elements": []}]}
    });
    let (_, report) = h
        .post(
            "/api/v1/card-templates/validate",
            json!({"name": "孤列", "kind": "card_message",
                   "card_message": {"source": "builder_json", "card_json": orphan_column}}),
        )
        .await;
    assert_eq!(report["valid"], false, "{report}");

    // 6. builder_template 已停用：新建被拒绝（历史模板仍可读取与预览）
    let (status, body) = h
        .post(
            "/api/v1/card-templates",
            json!({"name": "旧来源", "kind": "card_message",
                   "card_message": {"source": "builder_template", "template_id": "AAq123"}}),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("builder_template"),
        "{body}"
    );
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

    // 列表回显的地址必须换成新 token（旧地址不再出现），且主机仍是 public_base_url
    let (_, detail) = h.get(&format!("/api/v1/channels/{channel_id}")).await;
    let hook = detail["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == hook_id.as_str())
        .expect("轮换的 hook 应在列表里");
    let listed_url = hook["hook_url"].as_str().unwrap();
    assert_eq!(hook["hook_url_available"], true, "{hook}");
    assert!(
        listed_url.ends_with(&new_token),
        "列表必须回显轮换后的新地址: {listed_url}"
    );
    assert!(
        !listed_url.contains(&old_token),
        "列表不得再出现已失效的旧地址: {listed_url}"
    );
    assert!(listed_url.starts_with("http://alkh.test/api/v1/hooks/alertmanager/"));

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

    // 渠道页的测试消息**走完整链路并落库**：连续两次必须都成功。
    // 回归：测试分组 `alert_groups(hook_id, group_key)` 有唯一索引，第二次曾报
    // `UNIQUE constraint failed`（现在按自然键复用同一行）。
    let (status, second) = h
        .post(
            &format!("/api/v1/channels/{channel_id}/test-message"),
            json!({"sample": "firing"}),
        )
        .await;
    assert_eq!(status, 200, "渠道测试消息连续发送不应报唯一约束：{second}");
    assert_ne!(
        second["id"], delivery["id"],
        "每次渠道测试消息仍应是独立的投递记录: {second}"
    );
    // 两次都归属同一个「测试分组」行
    let (_, groups) = h.get("/api/v1/alert-groups").await;
    let test_groups = groups["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["group_key"].as_str() == Some("{}:{alertname=~\"High.*|DiskSpaceLow\"}"))
        .count();
    assert_eq!(
        test_groups, 1,
        "同一渠道的测试消息应复用同一个分组: {groups}"
    );

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

// ---------------------------------------------------------------------------
// R4c：卡片「忽略并 unpin」
// ---------------------------------------------------------------------------

async fn put_settings_with_token(h: &Harness, token: &str) {
    let resp = h
        .http
        .put(h.url("/api/v1/settings"))
        .bearer_auth(&h.token)
        .json(&json!({
            "lark_app_id": "cli_test",
            "app_secret": "test-secret-123",
            "verification_token": token,
            "lark_base_url": "https://open.feishu.cn",
            "retention": {"alert_group_days": 30, "delivery_days": 30, "audit_days": 14}
        }))
        .send()
        .await
        .expect("保存设置失败");
    assert_eq!(
        resp.status(),
        200,
        "保存设置（含 Verification Token）应成功"
    );
}

async fn post_card_callback(h: &Harness, body: Value) -> (reqwest::StatusCode, Value) {
    let resp = h
        .http
        .post(h.url("/api/v1/lark/card-callback"))
        .json(&body)
        .send()
        .await
        .expect("卡片回调请求失败");
    let status = resp.status();
    let parsed = resp.json().await.unwrap_or(Value::Null);
    (status, parsed)
}

fn ignore_event(token: &str, group_id: &str) -> Value {
    json!({
        "schema": "2.0",
        "header": {"event_id": "evt_1", "event_type": "card.action.trigger", "token": token, "app_id": "cli_test"},
        "event": {
            "operator": {"open_id": "ou_tester"},
            "action": {"tag": "button", "value": {"alkh_action": "ignore_unpin", "alert_group_id": group_id}},
            "context": {"open_message_id": "om_x", "open_chat_id": "oc_test_group"}
        }
    })
}

async fn wait_pin_ignored(h: &Harness, gid: &str, secs: u64) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let (_, g) = h.get(&format!("/api/v1/alert-groups/{gid}")).await;
        if g["pin_ignored"].as_bool() == Some(true) {
            return g;
        }
        if std::time::Instant::now() >= deadline {
            return g;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// R4c 主闭环：点按钮 → 撤销 Pin + 之后重复/恢复都不 Pin → 重开恢复 Pin。
#[tokio::test]
async fn card_ignore_unpin_suppresses_future_pins_and_reopen_restores() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;
    put_settings_with_token(&h, "test-verify-token-123").await;

    // firing → 发送并 Pin 当前卡片。
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["action"], "send_new");
    let gid = body["alert_group_id"].as_str().unwrap().to_string();
    let first_pin = {
        let s = h.fake.snapshot();
        assert_eq!(s.pinned.len(), 1, "开启 pin_unresolved 后 firing 应 Pin");
        s.pinned[0].clone()
    };

    // 点「忽略并 unpin」：异步执行，等标志落库且 Pin 被撤销。
    let (status, resp) = post_card_callback(&h, ignore_event("test-verify-token-123", &gid)).await;
    assert_eq!(status, 200, "{resp}");
    let g = wait_pin_ignored(&h, &gid, 3).await;
    assert_eq!(
        g["pin_ignored"].as_bool(),
        Some(true),
        "回调后应置 pin_ignored: {g}"
    );
    assert!(
        h.fake.snapshot().unpinned.contains(&first_pin),
        "应撤销分组已有 Pin"
    );
    assert!(
        group_pinned_messages(&h).await.is_empty(),
        "撤销后 Pin 记录应清空"
    );
    assert!(
        metric(&h, "alkh_pins_ignored_total").await >= 1,
        "忽略应计入指标"
    );

    // 重复通知（repeat=send）→ 发新卡但不 Pin。真实 AM 的重复通知间隔 > 60s，
    // 而幂等窗口是 60s；测试里同一 payload 会命中去重，故先清空去重表模拟「窗口已过」。
    put_policy_flag(&h, &channel_id, "repeat_policy", json!("send")).await;
    sqlx::query("DELETE FROM event_dedup")
        .execute(&h.pool)
        .await
        .expect("清空去重表失败");
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["event"], "repeated");
    assert_eq!(body["action"], "send_new");
    assert_eq!(
        h.fake.snapshot().pinned.len(),
        1,
        "忽略后重复通知不应再 Pin 新卡片"
    );

    // 全部恢复（即使开启 pin_resolved）→ 恢复卡也不 Pin。
    put_policy_flag(&h, &channel_id, "pin_resolved", json!(true)).await;
    let (_, body) = h.push(resolved_payload()).await;
    assert_eq!(body["event"], "resolved");
    assert_eq!(
        h.fake.snapshot().pinned.len(),
        1,
        "忽略后恢复通知不应 Pin 恢复卡片"
    );
    assert!(group_pinned_messages(&h).await.is_empty());

    // 重开（resolved → firing 视为新告警）→ 清除忽略标志并重新 Pin。
    sqlx::query("DELETE FROM event_dedup")
        .execute(&h.pool)
        .await
        .expect("清空去重表失败");
    let (_, body) = h.push(firing_payload(1)).await;
    assert_eq!(body["event"], "new_firing");
    let g = h.get(&format!("/api/v1/alert-groups/{gid}")).await.1;
    assert_eq!(
        g["pin_ignored"].as_bool(),
        Some(false),
        "重开后应清除忽略标志"
    );
    assert!(
        h.fake.snapshot().pinned.len() >= 2,
        "重开后新告警应重新 Pin"
    );
}

/// R4c：回调验签（challenge / 错误 token）。
#[tokio::test]
async fn card_callback_rejects_bad_token_and_answers_challenge() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    put_settings_with_token(&h, "test-verify-token-123").await;

    let (status, body) = post_card_callback(
        &h,
        json!({"type": "url_verification", "challenge": "abc123", "token": "test-verify-token-123"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["challenge"], "abc123");

    // 关键回归：飞书保存 URL 的校验体可能只有 `challenge`（无 type / 无 token）。
    // 即使已配置 token，也必须原样回 challenge，否则无法保存回调地址。
    for payload in [
        json!({"challenge": "only-challenge"}),
        json!({"type": "url_verification", "challenge": "no-token"}),
    ] {
        let (status, body) = post_card_callback(&h, payload).await;
        assert_eq!(status, 200, "challenge 校验必须 200: {body}");
        assert!(body.get("challenge").is_some(), "必须回 challenge: {body}");
    }

    let (status, _) = post_card_callback(&h, ignore_event("wrong-token-xxxx", "whatever")).await;
    assert_eq!(status, 403, "错误 Verification Token 必须被拒绝");
}

/// R4c：未配置 Verification Token 时事件回调拒绝（避免公开端点裸奔）。
#[tokio::test]
async fn card_callback_503_when_token_not_configured() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let (status, _) = post_card_callback(&h, ignore_event("any-token", "whatever")).await;
    assert_eq!(status, 503, "未配置 Verification Token 时应拒绝事件回调");
}

/// R4c：管理端「忽略并 unpin」/「恢复 Pin」反向出口。
#[tokio::test]
async fn admin_pin_ignore_and_restore_endpoints() {
    let mut h = start().await;
    let _ = h.create_channel().await;
    let channel_id = h.channel_id.clone().unwrap();
    put_policy_flag(&h, &channel_id, "pin_unresolved", json!(true)).await;

    let (_, body) = h.push(firing_payload(1)).await;
    let gid = body["alert_group_id"].as_str().unwrap().to_string();
    assert_eq!(h.fake.snapshot().pinned.len(), 1);

    let (status, out) = h
        .post(&format!("/api/v1/alert-groups/{gid}/pin-ignore"), json!({}))
        .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["found"], true);
    let g = h.get(&format!("/api/v1/alert-groups/{gid}")).await.1;
    assert_eq!(g["pin_ignored"].as_bool(), Some(true));
    assert!(group_pinned_messages(&h).await.is_empty());

    let resp = h
        .http
        .delete(h.url(&format!("/api/v1/alert-groups/{gid}/pin-ignore")))
        .bearer_auth(&h.token)
        .send()
        .await
        .expect("DELETE 失败");
    assert_eq!(resp.status(), 200);
    let g = h.get(&format!("/api/v1/alert-groups/{gid}")).await.1;
    assert_eq!(g["pin_ignored"].as_bool(), Some(false));
    assert!(
        h.fake.snapshot().pinned.len() >= 2,
        "恢复后应重新 Pin 当前活跃卡片"
    );
}
