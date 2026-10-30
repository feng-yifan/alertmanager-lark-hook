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

    // 1. 请求网址校验（保存回调地址时飞书发来的 challenge）。
    if value.get("type").and_then(|t| t.as_str()) == Some("url_verification") {
        match state.settings.callback_verification_token().await {
            Ok(Some(expected)) => {
                let got = value
                    .get("token")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default();
                if !ct_eq(got, &expected) {
                    return error_response(StatusCode::FORBIDDEN, "verification token mismatch");
                }
            }
            // 未配置 token 时允许完成 challenge（后续事件仍会被拒），便于先把地址配好。
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "读取回调 Verification Token 失败");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
            }
        }
        let challenge = value.get("challenge").cloned().unwrap_or(Value::Null);
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
    if action_value.get("alkh_action").and_then(|v| v.as_str()) != Some("ignore_unpin") {
        return (
            StatusCode::OK,
            Json(json!({"toast": {"type": "info", "content": "未识别的操作"}})),
        )
            .into_response();
    }
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

    // 5. 异步执行，先 ack（飞书要求 3s 内返回；取消 Pin 可能有多条，不能阻塞响应）。
    let state2 = state.clone();
    let gid = AlertGroupId::from_string(group_id);
    tokio::spawn(async move {
        match state2.intake.ignore_and_unpin(&gid, operator).await {
            Ok(out) if out.found => {
                tracing::info!(group_id = %gid, unpinned = out.unpinned, "卡片「忽略并 unpin」已执行");
            }
            Ok(_) => tracing::warn!(group_id = %gid, "忽略的分组不存在"),
            Err(e) => tracing::warn!(error = %e, "「忽略并 unpin」执行失败"),
        }
    });

    (
        StatusCode::OK,
        Json(json!({"toast": {"type": "success", "content": "已忽略，后续不再 Pin"}})),
    )
        .into_response()
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
