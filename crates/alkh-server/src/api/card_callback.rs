//! 飞书卡片回传交互端点（`card.action.trigger`）。
//!
//! 路径：`POST /api/v1/lark/card-callback`（公开端点，不在管理会话下）。
//!
//! 用途：处理告警卡片上「忽略并 unpin」按钮的点击。飞书在「事件与回调」里把回调
//! 请求打到本端点；本服务用应用设置里的 **Verification Token** 校验来源，随后异步
//! 执行 `IntakeService::ignore_and_unpin`，并立刻返回 toast（飞书要求 3s 内 ack）。
//!
//! 安全说明：采用 Verification Token 校验（事件明文、`header.token` 为应用验证标识）。
//! 若启用飞书 Encrypt Key 加密策略，事件 body 会被加密，本端点不处理（不引入 AES 解密）。
//! 因此部署时请保持「未启用 Encrypt Key」，仅用 Verification Token 校验。

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use alkh_domain::ids::AlertGroupId;
use alkh_domain::policy::NotificationPolicy;
use alkh_domain::template::CardSource;

use crate::state::AppState;

/// 处理飞书卡片回调。
pub async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    // 公开端点：先按来源 IP 限流。
    if let Some(retry_after) = state.callback_limiter.check(client_ip(&headers).as_deref()) {
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": {"code": "rate_limited", "message": "too many requests"}})),
        )
            .into_response();
        if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
            resp.headers_mut().insert(header::RETRY_AFTER, v);
        }
        return resp;
    }

    let value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid json"),
    };
    // 诊断：记录回调请求体的顶层字段名（不记录字段值，避免泄露 token），用于排查飞书发送的形态。
    if let Some(obj) = value.as_object() {
        let keys: Vec<&String> = obj.keys().collect();
        tracing::info!(keys = ?keys, "card-callback 请求体顶层字段");
    }

    // 1. 请求网址校验：飞书保存回调 URL 时会 POST 一个含 `challenge` 的请求
    //    （有时带 `type=url_verification` 与 `token`，有时仅有 `challenge`）。
    //    **无条件原样回 `challenge`**：该校验本身无副作用；若在此强制校验 token，
    //    会出现「token 未配置/配错 → 无法保存 URL」的死锁。真实事件（card.action.trigger）
    //    仍在下文严格校验 token。
    if let Some(challenge) = value.get("challenge") {
        return (StatusCode::OK, Json(json!({ "challenge": challenge }))).into_response();
    }

    // 2. 只处理卡片回传交互事件。
    let event_type = value
        .pointer("/header/event_type")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    if event_type != "card.action.trigger" {
        // 其它事件一律 200 忽略，避免飞书重投。
        return (StatusCode::OK, Json(json!({"code": 0}))).into_response();
    }

    // 3. 校验来源：Verification Token。
    let expected = match state.settings.callback_verification_token().await {
        Ok(Some(t)) => t,
        Ok(None) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "card callback 未配置 Verification Token",
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "读取回调 Verification Token 失败");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    };
    let got = value
        .pointer("/header/token")
        .and_then(|t| t.as_str())
        .or_else(|| value.get("token").and_then(|t| t.as_str()))
        .unwrap_or_default();
    if !ct_eq(got, &expected) {
        return error_response(StatusCode::FORBIDDEN, "token mismatch");
    }

    // 4. 解析按钮 action.value（可能是对象，也可能是 JSON 字符串）。
    let raw = value.pointer("/event/action/value");
    let action_value: Value = match raw {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let action = match action_value.get("alkh_action").and_then(|v| v.as_str()) {
        Some("ignore_unpin") => CallbackAction::Ignore,
        // R4c 可逆切换：分组已被忽略后按钮变为「恢复 Pin」，回调此动作。
        Some("restore_pin") => CallbackAction::Restore,
        _ => {
            return (
                StatusCode::OK,
                Json(json!({"toast": {"type": "info", "content": "未识别的操作"}})),
            )
                .into_response()
        }
    };
    let group_id = match action_value
        .get("alert_group_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(g) => g.to_string(),
        None => return error_response(StatusCode::BAD_REQUEST, "missing alert_group_id"),
    };
    let operator = value
        .pointer("/event/operator/open_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let gid = AlertGroupId::from_string(group_id);

    // 5. 即时渲染「切换后的卡片」并回给飞书（本地渲染、无外呼），让用户马上看到按钮变化。
    //    渲染失败不影响主流程，降级为只回 toast。
    let card_payload = render_toggled_card(&state, &gid, action).await;

    // 6. 真实副作用（撤销/恢复 Pin、审计、指标）异步执行：飞书要求 3s 内 ack，而
    //    Pin 调用可能多条多次外呼，不能阻塞响应。
    let state2 = state.clone();
    let gid2 = gid.clone();
    tokio::spawn(async move {
        let result = match action {
            CallbackAction::Ignore => state2.intake.ignore_and_unpin(&gid2, operator).await,
            CallbackAction::Restore => state2.intake.unignore_pin(&gid2).await,
        };
        match result {
            Ok(out) if out.found => {
                tracing::info!(group_id = %gid2, unpinned = out.unpinned, action = ?action, "卡片 Pin 操作已执行");
            }
            Ok(_) => tracing::warn!(group_id = %gid2, "操作的分组不存在"),
            Err(e) => tracing::warn!(error = %e, action = ?action, "卡片 Pin 操作执行失败"),
        }
    });

    let toast = match action {
        CallbackAction::Ignore => {
            json!({"type": "success", "content": "已忽略，后续不再 Pin"})
        }
        CallbackAction::Restore => json!({"type": "success", "content": "已恢复 Pin"}),
    };
    let mut body = json!({ "toast": toast });
    if let Some(card) = card_payload {
        body["card"] = card;
    }
    (StatusCode::OK, Json(body)).into_response()
}

/// 卡片按钮动作（R4c 可逆切换）。
#[derive(Clone, Copy, Debug)]
enum CallbackAction {
    /// 「忽略并 unpin」：撤销全部 Pin 并置 `pin_ignored`。
    Ignore,
    /// 「恢复 Pin」：清除 `pin_ignored` 并按策略重新 Pin。
    Restore,
}

/// 按目标动作渲染「切换后」的卡片内容，用于飞书回调的即时更新。
///
/// 仅本地渲染（加载分组/渠道/策略 + 套模板），不做任何飞书外呼，因此可在 3s ack
/// 预算内完成。任一环节失败都返回 `None`，调用方降级为只回 toast。
async fn render_toggled_card(
    state: &AppState,
    gid: &AlertGroupId,
    action: CallbackAction,
) -> Option<Value> {
    let group = match state.deps.groups.get(gid).await {
        Ok(Some(g)) => g,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "读取分组失败，跳过即时卡片更新");
            return None;
        }
    };
    let channel = match state.deps.channels.get(&group.channel_id).await {
        Ok(Some(c)) => c,
        _ => return None,
    };
    let policy = match state.deps.policies.get(&group.channel_id).await {
        Ok(Some(p)) => p,
        _ => NotificationPolicy::default_for(group.channel_id.clone()),
    };
    // 在内存副本上翻转标志，仅用于渲染出「切换后」的样子（真实落库在后台任务里做）。
    let mut view = group;
    match action {
        CallbackAction::Ignore => view.ignore_pin(),
        CallbackAction::Restore => view.clear_pin_ignored(),
    }
    let sender = state.sender().clone();
    let prepared = match sender
        .prepare_with_context(&view, &channel, &policy, None, None)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "即时渲染卡片失败，降级为仅 toast");
            return None;
        }
    };
    let content = prepared.content.to_content();
    Some(match prepared.card_source {
        Some(CardSource::BuilderTemplate) => content,
        _ => json!({"type": "raw", "data": content}),
    })
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"code": status.canonical_reason().unwrap_or("error"), "message": message}})),
    )
        .into_response()
}

/// 常量时间字符串比较（避免时序侧信道）。
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 取来源 IP：优先 `X-Forwarded-For` 第一段，其次 `X-Real-IP`。
fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_compares_bytes() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "ab"));
        assert!(ct_eq("", ""));
    }
}
