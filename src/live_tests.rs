//! The API client against a running LMOxide API (lmoxide's `tools/web-dev-backend.sh`):
//!
//! ```sh
//! LMOXIDE_TEST_API=http://127.0.0.1:8090 \
//! LMOXIDE_TEST_KEY=$(grep -o 'lmox_[0-9a-f]*' ../var/web-dev/demo-discord-bot-key.txt) cargo test live
//! ```
//!
//! Skipped without those variables. Uses the demo `admin` and `player` accounts and cleans up
//! after itself.

use serde_json::{Value, json};

use crate::api::{As, Client, CodePurpose, Registration};

struct Live {
    base: String,
    api: Client,
    http: reqwest::Client,
}

fn live() -> Option<Live> {
    let base = std::env::var("LMOXIDE_TEST_API").ok()?;
    let key = std::env::var("LMOXIDE_TEST_KEY").ok()?;
    Some(Live { api: Client::new(&base, key), base, http: reqwest::Client::new() })
}

impl Live {
    async fn post(&self, token: Option<&str>, path: &str, body: Value) -> (u16, Value) {
        let mut r = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        let resp = r.send().await.unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn login(&self, user: &str, password: &str) -> String {
        let (s, v) = self.post(None, "/v1/auth/login", json!({"username": user, "password": password})).await;
        assert_eq!(s, 200, "login {user}: {v}");
        v["token"].as_str().unwrap().to_string()
    }

    async fn delete_account(&self, admin: &str, id: u32) {
        let r = self.http.delete(format!("{}/v1/accounts/{id}", self.base)).bearer_auth(admin).send().await.unwrap();
        assert!(r.status().is_success(), "delete account {id}: {}", r.status());
    }
}

fn unique() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    // a plausible snowflake, different each run
    1_000_000_000_000_000_000 + t.as_micros() as u64 % 1_000_000_000_000
}

#[tokio::test]
async fn live_register_keys_and_codes() {
    let Some(l) = live() else {
        eprintln!("LMOXIDE_TEST_API/LMOXIDE_TEST_KEY not set: skipped");
        return;
    };
    let api = &l.api;
    assert_ne!(api.config().await.unwrap().registration, Registration::Closed);

    // register: an account linked to a Discord id, with an account key acting as it
    let did = unique();
    let username = format!("bot_test_{}", did % 1_000_000);
    let reg = api.register(did, &username, "first-password", 0).await.unwrap();
    assert_eq!(reg.link.discord_id, did.to_string());
    let me = api.me(&reg.key.secret).await.unwrap();
    assert_eq!(me.id, reg.link.account_id);
    assert_eq!(me.discord_id.as_deref(), Some(&*did.to_string()));
    assert!(api.links().await.unwrap().iter().any(|l| l.account_id == me.id));
    let dup = api.register(did, &format!("{username}x"), "first-password", 0).await.unwrap_err();
    assert_eq!(dup.code(), Some("conflict"));

    // the account key's rights follow the GM level
    let user = As::Account(reg.key.secret.clone());
    assert_eq!(api.broadcast(&user, "test").await.unwrap_err().code(), Some("forbidden"));
    // a contributor (2) still can't, a moderator (3) can
    api.set_gm_level(did, 2).await.unwrap();
    assert_eq!(api.broadcast(&user, "test").await.unwrap_err().code(), Some("forbidden"));
    let after = api.set_gm_level(did, 3).await.unwrap();
    assert_eq!(after.gm_level, 3);
    match api.broadcast(&user, "bot test").await {
        Ok(_) => {}
        Err(e) => assert_ne!(e.code(), Some("forbidden"), "GM 3 may broadcast: {e}"),
    }
    api.set_gm_level(did, 0).await.unwrap();

    // a new account key replaces the old one
    let key2 = api.account_key(did).await.unwrap();
    assert_eq!(api.me(&reg.key.secret).await.unwrap_err().code(), Some("unauthorized"));
    assert_eq!(api.me(&key2.secret).await.unwrap().id, me.id);

    // login code → a password_reset session → new password without the old one
    let code = api.code(did, CodePurpose::Login).await.unwrap();
    assert_eq!(code.code.len(), 9, "XXXX-XXXX: {}", code.code);
    let (s, v) = l.post(None, "/v1/auth/code", json!({"code": "WRNG-CODE"})).await;
    assert_eq!((s, v["error"]["code"].as_str()), (401, Some("bad_code")));
    let (s, sess) = l.post(None, "/v1/auth/code", json!({"code": code.code.to_lowercase().replace('-', "")})).await;
    assert_eq!(s, 200, "{sess}");
    assert_eq!(sess["password_reset"], true);
    let token = sess["token"].as_str().unwrap();
    let (s, v) = l.post(Some(token), "/v1/me/password", json!({"new_password": "second-password"})).await;
    assert_eq!(s, 204, "{v}");
    l.login(&username, "second-password").await;
    let (s, _) = l.post(None, "/v1/auth/code", json!({"code": code.code})).await;
    assert_eq!(s, 401, "codes work once");
    let link_code = api.code(did, CodePurpose::Link).await.unwrap_err();
    assert_eq!(link_code.code(), Some("conflict"), "already linked");

    // link code: the demo player links their account in the web UI
    let other = did + 1;
    assert!(api.link(other).await.unwrap().is_none());
    assert!(api.code(other, CodePurpose::Login).await.unwrap_err().is_not_found());
    let code = api.code(other, CodePurpose::Link).await.unwrap();
    let player = l.login("player", "player-password").await;
    let (s, v) = l.post(Some(&player), "/v1/me/discord", json!({"code": code.code})).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["discord_id"], other.to_string());
    let linked = api.link(other).await.unwrap().unwrap();
    assert_eq!(linked.username, "player");
    api.unlink(other).await.unwrap();
    assert!(api.link(other).await.unwrap().is_none());

    // unlinking ends the account key
    api.unlink(did).await.unwrap();
    assert_eq!(api.me(&key2.secret).await.unwrap_err().code(), Some("unauthorized"));

    let admin = l.login("admin", "admin-password").await;
    l.delete_account(&admin, me.id).await;
}
