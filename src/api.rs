//! A client for the LMOxide REST API (the routes the bot uses; the contract is lmoxide's
//! `docs/openapi.json`).
//!
//! Two kinds of credentials: the bot's own key (scope `discord`) for lookups, registrations,
//! links and syncs, and an account key per linked person, used when the bot acts for them so
//! the API decides what they may do and the audit log names them.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The API's own error answer.
    #[error("{message} ({code})")]
    Api { status: StatusCode, code: String, message: String },
    #[error("can't reach the LMOxide API: {0}")]
    Http(#[from] reqwest::Error),
}

impl ApiError {
    pub fn code(&self) -> Option<&str> {
        match self {
            ApiError::Api { code, .. } => Some(code),
            ApiError::Http(_) => None,
        }
    }

    pub fn status(&self) -> Option<StatusCode> {
        match self {
            ApiError::Api { status, .. } => Some(*status),
            ApiError::Http(_) => None,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.status() == Some(StatusCode::NOT_FOUND)
    }

    /// A message for the person who ran a command.
    pub fn describe(&self) -> String {
        match self.code() {
            Some("forbidden") => "You aren't allowed to do that.".into(),
            Some("unauthorized") => "The bot's credentials were refused by the server.".into(),
            Some("game_server_unavailable") => "The game server is offline.".into(),
            Some("no_control") => "The API isn't connected to a game server.".into(),
            Some("rate_limited") => "Too many requests; try again in a moment.".into(),
            Some("registration_closed") => "Registration is closed.".into(),
            Some(_) => match self {
                ApiError::Api { message, .. } => capitalize(message),
                ApiError::Http(_) => unreachable!(),
            },
            None => "The game server's API can't be reached right now.".into(),
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect::<String>() + if s.ends_with('.') { "" } else { "." },
        None => String::new(),
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

// ------------------------------------------------------------------ types

#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    pub game_server_online: bool,
    pub version: Option<String>,
    pub started_at: Option<i64>,
    pub players_online: Option<u32>,
    pub maintenance: Option<bool>,
    pub maintenance_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicPlayer {
    pub character_name: String,
    pub playfield: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Playfield {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Registration {
    Open,
    Discord,
    Closed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub registration: Registration,
    pub moderator_gm_level: u32,
    pub admin_gm_level: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ban {
    pub until: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharacterSummary {
    pub id: u32,
    pub name: String,
    pub playfield: u32,
    pub stars: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub id: u32,
    pub username: String,
    pub gm_level: u32,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
    pub ban: Option<Ban>,
    #[serde(default)]
    pub characters: Vec<CharacterSummary>,
    #[serde(default)]
    pub discord_id: Option<String>,
}

/// A character found by name, with its account.
#[derive(Debug, Clone, Deserialize)]
pub struct CharacterMatch {
    pub id: u32,
    pub name: String,
    pub mode: String,
    pub account_id: u32,
    pub username: String,
}

/// An account linked to a Discord account.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscordLink {
    pub discord_id: String,
    pub account_id: u32,
    pub username: String,
    pub gm_level: u32,
    pub banned: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatedApiKey {
    pub secret: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Registered {
    pub link: DiscordLink,
    pub key: CreatedApiKey,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodePurpose {
    /// Linking an existing account in the web UI.
    Link,
    /// Signing in to the web UI to choose a new password.
    Login,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Code {
    pub code: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Affected {
    pub affected: u32,
}

// ------------------------------------------------------------------ client

/// Who a request is made as.
#[derive(Debug, Clone)]
pub enum As {
    /// The bot's own key, optionally naming who it acts for (`X-On-Behalf-Of`).
    Bot { behalf: Option<String> },
    /// A linked account's key.
    Account(String),
}

impl As {
    pub const BOT: As = As::Bot { behalf: None };
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl Client {
    pub fn new(base: &str, key: String) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("lmoxide-discord-bot/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("HTTP client");
        Client { http, base: base.trim_end_matches('/').to_string(), key }
    }

    async fn send<T: DeserializeOwned>(&self, who: &As, method: Method, path: &str, body: Option<serde_json::Value>) -> Result<T> {
        let mut req = self.http.request(method, format!("{}{path}", self.base));
        req = match who {
            As::Bot { behalf } => {
                let r = req.bearer_auth(&self.key);
                match behalf {
                    Some(b) => r.header("X-On-Behalf-Of", b),
                    None => r,
                }
            }
            As::Account(key) => req.bearer_auth(key),
        };
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            if status == StatusCode::NO_CONTENT {
                return serde_json::from_value(serde_json::Value::Null).map_err(|e| ApiError::Api {
                    status,
                    code: "bad_response".into(),
                    message: e.to_string(),
                });
            }
            return Ok(resp.json().await?);
        }
        let text = resp.text().await.unwrap_or_default();
        Err(match serde_json::from_str::<ErrorBody>(&text) {
            Ok(b) => ApiError::Api { status, code: b.error.code, message: b.error.message },
            Err(_) => ApiError::Api { status, code: "http".into(), message: format!("HTTP {status}") },
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send(&As::BOT, Method::GET, path, None).await
    }

    // ---------------------------------------------------------------- public / read

    pub async fn status(&self) -> Result<Status> {
        self.get("/v1/status").await
    }

    pub async fn online(&self) -> Result<Vec<PublicPlayer>> {
        self.get("/v1/online").await
    }

    pub async fn playfields(&self) -> Result<Vec<Playfield>> {
        self.get("/v1/playfields").await
    }

    pub async fn config(&self) -> Result<ServerConfig> {
        self.get("/v1/config").await
    }

    /// The account with exactly this username (case-insensitive).
    pub async fn account_by_name(&self, username: &str) -> Result<Option<Account>> {
        let path = format!("/v1/accounts?search={}&limit=50", urlencode(username));
        let found: Vec<Account> = self.get(&path).await?;
        let Some(a) = found.into_iter().find(|a| a.username.eq_ignore_ascii_case(username)) else {
            return Ok(None);
        };
        self.account(a.id).await.map(Some)
    }

    /// Accounts whose username contains `search`, by username.
    pub async fn search_accounts(&self, search: &str, limit: u32) -> Result<Vec<Account>> {
        self.get(&format!("/v1/accounts?search={}&limit={limit}", urlencode(search))).await
    }

    /// Characters whose name contains `search`, by name.
    pub async fn search_characters(&self, search: &str, limit: u32) -> Result<Vec<CharacterMatch>> {
        self.get(&format!("/v1/characters?search={}&limit={limit}", urlencode(search))).await
    }

    pub async fn account(&self, id: u32) -> Result<Account> {
        self.get(&format!("/v1/accounts/{id}")).await
    }

    /// The account a key belongs to.
    pub async fn me(&self, key: &str) -> Result<Account> {
        self.send(&As::Account(key.to_string()), Method::GET, "/v1/me", None).await
    }

    // ---------------------------------------------------------------- Discord links

    pub async fn links(&self) -> Result<Vec<DiscordLink>> {
        self.get("/v1/discord/links").await
    }

    pub async fn link(&self, discord_id: u64) -> Result<Option<DiscordLink>> {
        match self.get(&format!("/v1/discord/links/{discord_id}")).await {
            Ok(l) => Ok(Some(l)),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn register(&self, discord_id: u64, username: &str, password: &str, gm_level: u32) -> Result<Registered> {
        let body = serde_json::json!({
            "discord_id": discord_id.to_string(), "username": username, "password": password, "gm_level": gm_level,
        });
        self.send(&As::BOT, Method::POST, "/v1/discord/accounts", Some(body)).await
    }

    pub async fn set_gm_level(&self, discord_id: u64, gm_level: u32) -> Result<DiscordLink> {
        let body = serde_json::json!({ "gm_level": gm_level });
        self.send(&As::BOT, Method::PATCH, &format!("/v1/discord/links/{discord_id}"), Some(body)).await
    }

    pub async fn unlink(&self, discord_id: u64) -> Result<()> {
        self.send(&As::BOT, Method::DELETE, &format!("/v1/discord/links/{discord_id}"), None).await
    }

    /// A new account key for a linked account (the previous one stops working).
    pub async fn account_key(&self, discord_id: u64) -> Result<CreatedApiKey> {
        self.send(&As::BOT, Method::POST, &format!("/v1/discord/links/{discord_id}/api-key"), None).await
    }

    pub async fn code(&self, discord_id: u64, purpose: CodePurpose) -> Result<Code> {
        let body = serde_json::json!({ "discord_id": discord_id.to_string(), "purpose": purpose });
        self.send(&As::BOT, Method::POST, "/v1/discord/codes", Some(body)).await
    }

    // ---------------------------------------------------------------- moderation

    pub async fn kick(&self, who: &As, account_id: u32, reason: Option<&str>) -> Result<Affected> {
        let body = serde_json::json!({ "reason": reason });
        self.send(who, Method::POST, &format!("/v1/players/{account_id}/kick"), Some(body)).await
    }

    pub async fn ban(&self, who: &As, account_id: u32, days: Option<u32>, reason: Option<&str>) -> Result<Account> {
        let body = serde_json::json!({ "days": days, "reason": reason });
        self.send(who, Method::POST, &format!("/v1/accounts/{account_id}/ban"), Some(body)).await
    }

    pub async fn unban(&self, who: &As, account_id: u32) -> Result<Account> {
        self.send(who, Method::POST, &format!("/v1/accounts/{account_id}/unban"), None).await
    }

    pub async fn broadcast(&self, who: &As, message: &str) -> Result<Affected> {
        let body = serde_json::json!({ "message": message });
        self.send(who, Method::POST, "/v1/broadcast", Some(body)).await
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_read_well() {
        let e = ApiError::Api { status: StatusCode::CONFLICT, code: "conflict".into(), message: "username taken".into() };
        assert_eq!(e.describe(), "Username taken.");
        let e = ApiError::Api { status: StatusCode::FORBIDDEN, code: "forbidden".into(), message: "x".into() };
        assert_eq!(e.describe(), "You aren't allowed to do that.");
    }

    #[test]
    fn query_escaping() {
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
    }
}
