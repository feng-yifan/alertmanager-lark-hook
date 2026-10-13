//! 鉴权：会话 Cookie / Bearer + 管理员提取器。

use alkh_domain::audit::AuditActor;
use alkh_domain::auth::AdminUser;
use axum::extract::{FromRequestParts, State};
use axum::http::{header, request::Parts, HeaderMap};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use alkh_app::AppError;

/// 已认证的管理员。
#[derive(Debug, Clone)]
pub struct Admin {
    pub user: AdminUser,
    pub token: String,
}

impl Admin {
    pub fn actor(&self) -> AuditActor {
        AuditActor::admin(self.user.username.clone())
    }
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)
            .or_else(|| cookie_token(parts))
            .ok_or(ApiError(AppError::Unauthenticated))?;
        let user = state
            .auth
            .session_user(&token)
            .await
            .map_err(ApiError::from)?;
        // 初始密码可能已经出现在启动日志里，必须先在服务端强制改掉。
        // 此前「必须改密」只是前端提示，用初始口令就能直接调用全部管理接口。
        if user.must_change_password && !password_change_allowed(parts.uri.path()) {
            return Err(ApiError(AppError::Forbidden(
                "当前账号仍在使用初始密码，请先修改密码后再使用其它管理功能".into(),
            )));
        }
        Ok(Admin { user, token })
    }
}

/// 强制改密期间仍可访问的端点 —— 否则无法完成改密本身。
///
/// 注意：axum 的 `nest` 会把挂载前缀剥掉，所以这里拿到的 path 是
/// `/auth/session` 这种相对路径（不是 `/api/v1/auth/session`）。
/// 用后缀匹配，将来调整挂载点也不会失效。
fn password_change_allowed(path: &str) -> bool {
    const ALLOWED: [&str; 3] = ["/auth/password", "/auth/logout", "/auth/session"];
    let path = path.trim_end_matches('/');
    ALLOWED.iter().any(|allowed| path.ends_with(allowed))
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn cookie_token(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == Config::session_cookie_name())
        .map(|(_, v)| v.to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub username: String,
    pub must_change_password: bool,
    /// 便于非浏览器客户端使用 `Authorization: Bearer`。
    pub token: String,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Response> {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let ip = client_ip(&headers);

    let (user, token) = state
        .auth
        .login(&req.username, &req.password, user_agent, ip)
        .await
        .map_err(ApiError::from)?;

    let ttl = 60 * 60 * 24;
    let mut cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={ttl}",
        Config::session_cookie_name(),
        token
    );
    if state.config.cookie_secure {
        cookie.push_str("; Secure");
    }

    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(LoginResponse {
            username: user.username,
            must_change_password: user.must_change_password,
            token,
        }),
    )
        .into_response())
}

pub async fn logout(State(state): State<AppState>, admin: Admin) -> ApiResult<Response> {
    let actor = admin.actor();
    state
        .auth
        .logout(&admin.token, &actor)
        .await
        .map_err(ApiError::from)?;

    let mut cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        Config::session_cookie_name()
    );
    if state.config.cookie_secure {
        cookie.push_str("; Secure");
    }
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({"ok": true}))).into_response())
}

pub async fn session(admin: Admin) -> impl IntoResponse {
    Json(json!({
        "authenticated": true,
        "username": admin.user.username,
        "must_change_password": admin.user.must_change_password,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    admin: Admin,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiResult<Response> {
    let actor = admin.actor();
    state
        .auth
        .change_password(
            &admin.user.id,
            &req.current_password,
            &req.new_password,
            &actor,
        )
        .await
        .map_err(ApiError::from)?;

    // 所有会话已失效，清理 Cookie。
    // 注意要与登录时写入的属性保持一致（尤其是 Secure），否则在 HTTPS 部署下
    // 浏览器可能忽略这条删除指令、把旧 Cookie 留在本地。
    let mut cookie = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        Config::session_cookie_name()
    );
    if state.config.cookie_secure {
        cookie.push_str("; Secure");
    }
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(json!({"ok": true, "reauth_required": true})),
    )
        .into_response())
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn parts_with(headers: &[(&str, &str)]) -> Parts {
        let mut builder = Request::builder().uri("/");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn bearer_token_parsed() {
        let parts = parts_with(&[("authorization", "Bearer abc123")]);
        assert_eq!(bearer_token(&parts).as_deref(), Some("abc123"));
    }

    #[test]
    fn cookie_token_parsed_among_others() {
        let parts = parts_with(&[("cookie", "foo=1; alkh_session=tok; bar=2")]);
        assert_eq!(cookie_token(&parts).as_deref(), Some("tok"));
    }

    #[test]
    fn missing_credentials_yield_none() {
        let parts = parts_with(&[]);
        assert!(bearer_token(&parts).is_none());
        assert!(cookie_token(&parts).is_none());
    }
}
