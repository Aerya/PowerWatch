use crate::state::AppState;
use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const SESSION_COOKIE: &str = "powerwatch_session";
const SESSION_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;
const LOGIN_WINDOW: Duration = Duration::from_secs(5 * 60);
const LOGIN_MAX_FAILURES: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    username: String,
    password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSession {
    token_hash: String,
    csrf_token: String,
    created_at: i64,
    expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApiToken {
    id: String,
    name: String,
    token_hash: String,
    scope: String,
    created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AuthStore {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<Account>,
    #[serde(default)]
    sessions: Vec<StoredSession>,
    #[serde(default)]
    api_tokens: Vec<ApiToken>,
}

#[derive(Clone)]
pub struct AuthService {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    enabled: bool,
    path: Option<PathBuf>,
    setup_secret: Option<String>,
    store: RwLock<AuthStore>,
    failures: Mutex<HashMap<String, VecDeque<Instant>>>,
}

#[derive(Debug, Serialize)]
pub struct AuthStatus {
    enabled: bool,
    setup_required: bool,
    authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csrf_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    username: String,
    password: String,
    setup_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
pub struct PasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiTokenView {
    id: String,
    name: String,
    scope: String,
    created_at: i64,
    last_used_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SecuritySettings {
    username: String,
    active_sessions: usize,
    api_tokens: Vec<ApiTokenView>,
}

#[derive(Debug, Serialize)]
pub struct CreatedToken {
    token: String,
    token_info: ApiTokenView,
}

enum Authentication {
    Disabled,
    Session { csrf_token: String },
    ApiToken,
    Missing,
}

pub fn default_auth_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    FsPath::new(&home).join(".local/share/powerwatch/auth.json")
}

fn setup_secret_from_environment() -> Result<Option<String>, String> {
    if let Ok(path) = std::env::var("POWERWATCH_AUTH_SETUP_TOKEN_FILE") {
        let secret = std::fs::read_to_string(path.trim())
            .map_err(|error| format!("cannot read POWERWATCH_AUTH_SETUP_TOKEN_FILE: {error}"))?;
        return Ok(Some(secret.trim().to_string()));
    }
    Ok(std::env::var("POWERWATCH_AUTH_SETUP_TOKEN")
        .ok()
        .map(|value| value.trim().to_string()))
}

impl AuthService {
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(AuthInner {
                enabled: false,
                path: None,
                setup_secret: None,
                store: RwLock::new(AuthStore::default()),
                failures: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn load(enabled: bool, path: PathBuf) -> Result<Self, String> {
        let mut store = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            serde_json::from_str::<AuthStore>(&raw)
                .map_err(|error| format!("invalid {}: {error}", path.display()))?
        } else {
            AuthStore::default()
        };
        store
            .sessions
            .retain(|session| session.expires_at > Utc::now().timestamp());

        let setup_secret = setup_secret_from_environment()?;
        if enabled && store.account.is_none() {
            let valid = setup_secret
                .as_deref()
                .is_some_and(|secret| secret.chars().count() >= 16);
            if !valid {
                return Err("authentication is enabled without an existing account: set POWERWATCH_AUTH_SETUP_TOKEN (at least 16 characters) or POWERWATCH_AUTH_SETUP_TOKEN_FILE before the first start".to_string());
            }
        }

        Ok(Self {
            inner: Arc::new(AuthInner {
                enabled,
                path: Some(path),
                setup_secret,
                store: RwLock::new(store),
                failures: Mutex::new(HashMap::new()),
            }),
        })
    }

    #[cfg(test)]
    pub fn load_with_setup_secret(
        enabled: bool,
        path: PathBuf,
        setup_secret: Option<String>,
    ) -> Result<Self, String> {
        let mut store = if path.exists() {
            serde_json::from_str::<AuthStore>(
                &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?
        } else {
            AuthStore::default()
        };
        store
            .sessions
            .retain(|session| session.expires_at > Utc::now().timestamp());
        if enabled
            && store.account.is_none()
            && setup_secret.as_deref().is_none_or(|s| s.len() < 16)
        {
            return Err("missing setup secret".to_string());
        }
        Ok(Self {
            inner: Arc::new(AuthInner {
                enabled,
                path: Some(path),
                setup_secret,
                store: RwLock::new(store),
                failures: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled
    }

    pub fn setup_required(&self) -> bool {
        self.inner.enabled && self.inner.store.read().unwrap().account.is_none()
    }

    fn persist(&self, store: &AuthStore) -> Result<(), String> {
        let Some(path) = self.inner.path.as_ref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temporary = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(store).map_err(|error| error.to_string())?;
        std::fs::write(&temporary, data).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        std::fs::rename(&temporary, path).map_err(|error| error.to_string())
    }

    fn status_for_headers(&self, headers: &HeaderMap) -> AuthStatus {
        if !self.enabled() {
            return AuthStatus {
                enabled: false,
                setup_required: false,
                authenticated: true,
                username: None,
                csrf_token: None,
            };
        }
        let store = self.inner.store.read().unwrap();
        let session = cookie_value(headers, SESSION_COOKIE)
            .map(|token| token_hash(&token))
            .and_then(|hash| valid_session(&store, &hash).cloned());
        AuthStatus {
            enabled: true,
            setup_required: store.account.is_none(),
            authenticated: session.is_some(),
            username: session
                .as_ref()
                .and_then(|_| store.account.as_ref().map(|a| a.username.clone())),
            csrf_token: session.map(|value| value.csrf_token),
        }
    }

    fn create_session(&self, store: &mut AuthStore) -> (String, String) {
        let raw_token = random_token(32);
        let csrf_token = random_token(32);
        let now = Utc::now().timestamp();
        store.sessions.retain(|session| session.expires_at > now);
        store.sessions.push(StoredSession {
            token_hash: token_hash(&raw_token),
            csrf_token: csrf_token.clone(),
            created_at: now,
            expires_at: now + SESSION_LIFETIME_SECONDS,
        });
        (raw_token, csrf_token)
    }

    fn authenticate(&self, headers: &HeaderMap) -> Authentication {
        if !self.enabled() {
            return Authentication::Disabled;
        }
        if let Some(value) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            if let Some(raw) = value.strip_prefix("Bearer ") {
                let hash = token_hash(raw.trim());
                let now = Utc::now().timestamp();
                let mut store = self.inner.store.write().unwrap();
                if let Some(token) = store
                    .api_tokens
                    .iter_mut()
                    .find(|token| constant_time_eq(&token.token_hash, &hash))
                {
                    if token
                        .last_used_at
                        .is_none_or(|last_used| now - last_used >= 60)
                    {
                        token.last_used_at = Some(now);
                        let _ = self.persist(&store);
                    }
                    return Authentication::ApiToken;
                }
                return Authentication::Missing;
            }
        }
        let Some(raw) = cookie_value(headers, SESSION_COOKIE) else {
            return Authentication::Missing;
        };
        let hash = token_hash(&raw);
        let store = self.inner.store.read().unwrap();
        valid_session(&store, &hash)
            .map(|session| Authentication::Session {
                csrf_token: session.csrf_token.clone(),
            })
            .unwrap_or(Authentication::Missing)
    }

    fn rate_limited(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut failures = self.inner.failures.lock().unwrap();
        let values = failures.entry(key.to_string()).or_default();
        while values
            .front()
            .is_some_and(|instant| now.duration_since(*instant) > LOGIN_WINDOW)
        {
            values.pop_front();
        }
        values.len() >= LOGIN_MAX_FAILURES
    }

    fn record_failure(&self, key: &str) {
        self.inner
            .failures
            .lock()
            .unwrap()
            .entry(key.to_string())
            .or_default()
            .push_back(Instant::now());
    }

    fn clear_failures(&self, key: &str) {
        self.inner.failures.lock().unwrap().remove(key);
    }
}

fn argon2() -> Argon2<'static> {
    let params = Params::new(19_456, 2, 1, None).expect("valid Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded)
        .ok()
        .is_some_and(|hash| argon2().verify_password(password.as_bytes(), &hash).is_ok())
}

fn validate_username(username: &str) -> Result<String, String> {
    let username = username.trim();
    if username.is_empty()
        || username.chars().count() > 64
        || username.chars().any(char::is_control)
    {
        return Err("username must contain 1 to 64 printable characters".to_string());
    }
    Ok(username.to_string())
}

fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < 12 {
        return Err("password must contain at least 12 characters".to_string());
    }
    if password.len() > 1024 {
        return Err("password is too long".to_string());
    }
    Ok(())
}

fn random_token(bytes: usize) -> String {
    let mut value = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut value);
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn token_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_string()))
}

fn valid_session<'a>(store: &'a AuthStore, hash: &str) -> Option<&'a StoredSession> {
    let now = Utc::now().timestamp();
    store
        .sessions
        .iter()
        .find(|session| session.expires_at > now && constant_time_eq(&session.token_hash, hash))
}

fn secure_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
        })
        || headers
            .get("forwarded")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .any(|part| part.trim().eq_ignore_ascii_case("proto=https"))
            })
}

fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_LIFETIME_SECONDS}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn expired_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

fn response_with_cookie(status: AuthStatus, cookie: String) -> Response {
    let mut response = Json(status).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("generated cookie is valid"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn status(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let mut response = Json(state.auth.status_for_headers(&headers)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetupRequest>,
) -> Response {
    if !state.auth.enabled() {
        return json_error(StatusCode::NOT_FOUND, "authentication is disabled");
    }
    let key = "setup";
    if state.auth.rate_limited(key) {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
    }
    let expected = state.auth.inner.setup_secret.as_deref().unwrap_or_default();
    if !constant_time_eq(request.setup_token.trim(), expected) {
        state.auth.record_failure(key);
        return json_error(StatusCode::UNAUTHORIZED, "invalid setup token");
    }
    let username = match validate_username(&request.username) {
        Ok(username) => username,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, &error),
    };
    if let Err(error) = validate_password(&request.password) {
        return json_error(StatusCode::BAD_REQUEST, &error);
    }
    let password_hash = match hash_password(&request.password) {
        Ok(hash) => hash,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    let (raw_token, csrf_token) = {
        let mut store = state.auth.inner.store.write().unwrap();
        if store.account.is_some() {
            return json_error(
                StatusCode::CONFLICT,
                "the administrator account already exists",
            );
        }
        store.account = Some(Account {
            username: username.clone(),
            password_hash,
        });
        let session = state.auth.create_session(&mut store);
        if let Err(error) = state.auth.persist(&store) {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error);
        }
        session
    };
    state.auth.clear_failures(key);
    response_with_cookie(
        AuthStatus {
            enabled: true,
            setup_required: false,
            authenticated: true,
            username: Some(username),
            csrf_token: Some(csrf_token),
        },
        session_cookie(&raw_token, secure_request(&headers)),
    )
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Response {
    if !state.auth.enabled() {
        return json_error(StatusCode::NOT_FOUND, "authentication is disabled");
    }
    if state.auth.setup_required() {
        return json_error(
            StatusCode::PRECONDITION_REQUIRED,
            "the administrator account must be created first",
        );
    }
    let key = request.username.trim().to_ascii_lowercase();
    if state.auth.rate_limited(&key) {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too many attempts; try again later",
        );
    }
    let valid = {
        let store = state.auth.inner.store.read().unwrap();
        store.account.as_ref().is_some_and(|account| {
            let username_matches = constant_time_eq(&account.username, request.username.trim());
            let password_matches = verify_password(&request.password, &account.password_hash);
            username_matches && password_matches
        })
    };
    if !valid {
        state.auth.record_failure(&key);
        return json_error(StatusCode::UNAUTHORIZED, "invalid username or password");
    }
    let (raw_token, csrf_token, username) = {
        let mut store = state.auth.inner.store.write().unwrap();
        let username = store.account.as_ref().unwrap().username.clone();
        let session = state.auth.create_session(&mut store);
        if let Err(error) = state.auth.persist(&store) {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error);
        }
        (session.0, session.1, username)
    };
    state.auth.clear_failures(&key);
    response_with_cookie(
        AuthStatus {
            enabled: true,
            setup_required: false,
            authenticated: true,
            username: Some(username),
            csrf_token: Some(csrf_token),
        },
        session_cookie(&raw_token, secure_request(&headers)),
    )
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(raw) = cookie_value(&headers, SESSION_COOKIE) {
        let hash = token_hash(&raw);
        let mut store = state.auth.inner.store.write().unwrap();
        store
            .sessions
            .retain(|session| !constant_time_eq(&session.token_hash, &hash));
        if let Err(error) = state.auth.persist(&store) {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error);
        }
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&expired_cookie(secure_request(&headers))).unwrap(),
    );
    response
}

pub async fn security_settings(State(state): State<AppState>) -> Response {
    let store = state.auth.inner.store.read().unwrap();
    let Some(account) = store.account.as_ref() else {
        return json_error(
            StatusCode::PRECONDITION_REQUIRED,
            "account setup is incomplete",
        );
    };
    let now = Utc::now().timestamp();
    let mut response = Json(SecuritySettings {
        username: account.username.clone(),
        active_sessions: store
            .sessions
            .iter()
            .filter(|session| session.expires_at > now)
            .count(),
        api_tokens: store.api_tokens.iter().map(ApiTokenView::from).collect(),
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PasswordRequest>,
) -> Response {
    if let Err(error) = validate_password(&request.new_password) {
        return json_error(StatusCode::BAD_REQUEST, &error);
    }
    let new_hash = match hash_password(&request.new_password) {
        Ok(hash) => hash,
        Err(error) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    let mut store = state.auth.inner.store.write().unwrap();
    let Some(account) = store.account.as_mut() else {
        return json_error(
            StatusCode::PRECONDITION_REQUIRED,
            "account setup is incomplete",
        );
    };
    if !verify_password(&request.current_password, &account.password_hash) {
        return json_error(StatusCode::UNAUTHORIZED, "current password is incorrect");
    }
    account.password_hash = new_hash;
    store.sessions.clear();
    if let Err(error) = state.auth.persist(&store) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&expired_cookie(secure_request(&headers))).unwrap(),
    );
    response
}

pub async fn revoke_sessions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let mut store = state.auth.inner.store.write().unwrap();
    store.sessions.clear();
    if let Err(error) = state.auth.persist(&store) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error);
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&expired_cookie(secure_request(&headers))).unwrap(),
    );
    response
}

pub async fn create_api_token(
    State(state): State<AppState>,
    Json(request): Json<TokenRequest>,
) -> Response {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "token name must contain 1 to 80 printable characters",
        );
    }
    let raw = format!("pw_{}", random_token(32));
    let token = ApiToken {
        id: random_token(8),
        name: name.to_string(),
        token_hash: token_hash(&raw),
        scope: "read".to_string(),
        created_at: Utc::now().timestamp(),
        last_used_at: None,
    };
    let view = ApiTokenView::from(&token);
    let mut store = state.auth.inner.store.write().unwrap();
    store.api_tokens.push(token);
    if let Err(error) = state.auth.persist(&store) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &error);
    }
    let mut response = (
        StatusCode::CREATED,
        Json(CreatedToken {
            token: raw,
            token_info: view,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn revoke_api_token(Path(id): Path<String>, State(state): State<AppState>) -> Response {
    let mut store = state.auth.inner.store.write().unwrap();
    let before = store.api_tokens.len();
    store.api_tokens.retain(|token| token.id != id);
    if store.api_tokens.len() == before {
        return json_error(StatusCode::NOT_FOUND, "API token not found");
    }
    match state.auth.persist(&store) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    }
}

impl From<&ApiToken> for ApiTokenView {
    fn from(token: &ApiToken) -> Self {
        Self {
            id: token.id.clone(),
            name: token.name.clone(),
            scope: token.scope.clone(),
            created_at: token.created_at,
            last_used_at: token.last_used_at,
        }
    }
}

fn bearer_route_allowed(method: &Method, path: &str) -> bool {
    matches!(*method, Method::GET | Method::HEAD)
        && matches!(
            path,
            "/api/snapshot" | "/api/history" | "/api/history/range" | "/api/instance"
        )
}

pub async fn require_auth(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let authentication = state.auth.authenticate(request.headers());
    match authentication {
        Authentication::Disabled => next.run(request).await,
        Authentication::ApiToken
            if bearer_route_allowed(request.method(), request.uri().path()) =>
        {
            next.run(request).await
        }
        Authentication::ApiToken => json_error(
            StatusCode::FORBIDDEN,
            "the API token is read-only and cannot access this endpoint",
        ),
        Authentication::Session { csrf_token } => {
            let safe = matches!(
                *request.method(),
                Method::GET | Method::HEAD | Method::OPTIONS
            );
            if safe {
                return next.run(request).await;
            }
            let provided = request
                .headers()
                .get("x-csrf-token")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            if !constant_time_eq(provided, &csrf_token) {
                return json_error(StatusCode::FORBIDDEN, "missing or invalid CSRF token");
            }
            next.run(request).await
        }
        Authentication::Missing => json_error(StatusCode::UNAUTHORIZED, "authentication required"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_is_argon2id_and_verifies() {
        let hash = hash_password("a strong password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("a strong password", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn auth_store_survives_restart_with_sessions_and_without_plain_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let service = AuthService::load_with_setup_secret(
            true,
            path.clone(),
            Some("bootstrap-secret-123".into()),
        )
        .unwrap();
        let mut store = service.inner.store.write().unwrap();
        store.account = Some(Account {
            username: "admin".into(),
            password_hash: hash_password("a strong password").unwrap(),
        });
        let (raw_session, _) = service.create_session(&mut store);
        let raw_api = "pw_machine-token";
        store.api_tokens.push(ApiToken {
            id: "id".into(),
            name: "Hub".into(),
            token_hash: token_hash(raw_api),
            scope: "read".into(),
            created_at: Utc::now().timestamp(),
            last_used_at: None,
        });
        service.persist(&store).unwrap();
        drop(store);

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("a strong password"));
        assert!(!raw.contains(&raw_session));
        assert!(!raw.contains(raw_api));

        let restarted = AuthService::load_with_setup_secret(true, path, None).unwrap();
        let store = restarted.inner.store.read().unwrap();
        assert!(valid_session(&store, &token_hash(&raw_session)).is_some());
        assert!(store
            .api_tokens
            .iter()
            .any(|token| token.token_hash == token_hash(raw_api)));
    }

    #[test]
    fn setup_requires_an_external_bootstrap_secret() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            AuthService::load_with_setup_secret(true, dir.path().join("auth.json"), None).is_err()
        );
    }

    #[test]
    fn rate_limit_stops_repeated_failures() {
        let service = AuthService::disabled();
        for _ in 0..LOGIN_MAX_FAILURES {
            service.record_failure("admin");
        }
        assert!(service.rate_limited("admin"));
        service.clear_failures("admin");
        assert!(!service.rate_limited("admin"));
    }

    #[test]
    fn https_proxy_requests_receive_secure_cookies() {
        assert!(session_cookie("token", true).contains("; Secure"));
        assert!(!session_cookie("token", false).contains("; Secure"));
    }
}
