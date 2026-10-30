//! Game accounts for Discord members: register, link, password reset, account info.

use std::time::Duration;

use poise::{Modal, serenity_prelude as serenity};

use super::{as_caller, clean, complete_username, date, member_roles, refuse, relative, say, user_id};
use crate::{Context, Data, Error, api, sync};

#[derive(Debug, Modal)]
#[name = "Create your LMOxide account"]
struct RegisterForm {
    #[name = "Username (what you log in with)"]
    #[placeholder = "3-32 letters, digits, _ - ."]
    #[min_length = 3]
    #[max_length = 32]
    username: String,
    #[name = "Password"]
    #[placeholder = "8 characters or more"]
    #[min_length = 8]
    #[max_length = 128]
    password: String,
    #[name = "Password again"]
    #[min_length = 8]
    #[max_length = 128]
    confirm: String,
}

/// The API's username rule: 3-32 ASCII letters, digits, `_`, `-` or `.`.
fn valid_username(name: &str) -> bool {
    (3..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

/// Create a game account linked to your Discord account.
#[poise::command(slash_command, guild_only)]
pub async fn register(ctx: poise::ApplicationContext<'_, Data, Error>) -> Result<(), Error> {
    let c = Context::Application(ctx);
    let data = c.data();
    let me = user_id(c);
    let roles = member_roles(c).await?;
    if let Some(link) = data.api.link(me).await? {
        return refuse(format!(
            "You already have an account: `{}`. Forgot its password? Use `/reset-password`.",
            clean(&link.username)
        ));
    }
    if data.api.config().await?.registration == api::Registration::Closed {
        return refuse("Registration is closed at the moment.");
    }
    let Some(form) = RegisterForm::execute(ctx).await? else { return Ok(()) };
    let username = form.username.trim();
    if !valid_username(username) {
        return refuse("Usernames are 3-32 letters, digits, `_`, `-` or `.`. Run `/register` again.");
    }
    if form.password != form.confirm {
        return refuse("The two passwords differ. Run `/register` again.");
    }
    let cfg = data.cfg();
    let gm = cfg.gm_level_for(Some(&roles));
    let reg = data.api.register(me, username, &form.password, gm).await?;
    data.store.set_key(me, reg.link.account_id, &reg.key.secret)?;
    tracing::info!(user = %reg.link.username, discord = me, gm, "registered");
    // the Alpha Tester role and anything else the sync does
    if let Err(e) = sync::member(c.serenity_context(), data, me, Some(&roles)).await {
        tracing::warn!(discord = me, "sync after registering: {e:#}");
    }
    let mut text = format!(
        "Your account **{}** is ready. Log in to the game with it. Its page on the web: <{}/me>.",
        clean(&reg.link.username),
        cfg.webui_url
    );
    if gm > 0 {
        text += &format!("\nYour {} role gives it GM level {gm}.", cfg.gm_role_name(gm).unwrap_or("Discord"));
    }
    text += "\nForgot your password later? `/reset-password`.";
    say(c, text).await
}

/// Link the game account you already have to your Discord account (the code comes by DM).
#[poise::command(slash_command, guild_only)]
pub async fn link(ctx: Context<'_>) -> Result<(), Error> {
    let data = ctx.data();
    let me = user_id(ctx);
    member_roles(ctx).await?;
    if let Some(link) = data.api.link(me).await? {
        return refuse(format!("Your Discord account is already linked to `{}`.", clean(&link.username)));
    }
    let code = data.api.code(me, api::CodePurpose::Link).await?;
    let cfg = data.cfg();
    let text = format!(
        "Your code to link your LMOxide account: **{code}** (it expires {expires}).\n\
         Open <{web}/me?link_code={code}> and sign in with the account you want to link, or enter the code \
         on your account page.\nIf you didn't ask for this, ignore this message.",
        code = code.code,
        expires = relative(code.expires_at),
        web = cfg.webui_url,
    );
    dm(ctx, text).await?;
    watch_link(ctx.serenity_context().clone(), data.clone(), ctx.author().id, code.expires_at);
    say(ctx, "I sent you a DM with a code. Enter it in the web UI while signed in to your game account.").await
}

/// Waits for a `/link` code to be used in the web UI (the API has no events), then makes the
/// account key, syncs the GM level and roles, and says so.
fn watch_link(ctx: serenity::Context, data: Data, user: serenity::UserId, until: i64) {
    let me = user.get();
    if !data.pending_links.lock().unwrap().insert(me) {
        return; // already watching; the new code replaced the old one, keep the longer wait
    }
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs((until - now()).clamp(0, 3600) as u64 + 5);
        let mut found = None;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(5)).await;
            match data.api.link(me).await {
                Ok(Some(l)) => {
                    found = Some(l);
                    break;
                }
                Ok(None) => {}
                Err(e) => tracing::debug!(discord = me, "waiting for link: {e}"),
            }
        }
        data.pending_links.lock().unwrap().remove(&me);
        let Some(link) = found else { return };
        tracing::info!(user = %link.username, discord = me, "linked");
        let report = sync::member_by_id(&ctx, &data, me).await;
        if let Err(e) = &report {
            tracing::warn!(discord = me, "sync after linking: {e:#}");
        }
        let mut text = format!("Your Discord account is now linked to the game account **{}**.", clean(&link.username));
        if let Ok(r) = report
            && let Some((_, _, gm)) = r.gm_changes.first()
            && *gm > 0
        {
            text += &format!(" Your Discord roles give it GM level {gm}.");
        }
        let _ = user.direct_message(&ctx, serenity::CreateMessage::new().content(text)).await;
    });
}

/// Get a one-time sign-in code by DM, to choose a new password in the web UI.
#[poise::command(slash_command, guild_only, rename = "reset-password")]
pub async fn reset_password(ctx: Context<'_>) -> Result<(), Error> {
    let data = ctx.data();
    let me = user_id(ctx);
    let Some(link) = data.api.link(me).await? else {
        return refuse("Your Discord account isn't linked to a game account. `/register` makes one.");
    };
    if link.banned {
        return refuse("That account is banned.");
    }
    let code = data.api.code(me, api::CodePurpose::Login).await?;
    let text = format!(
        "To choose a new password for **{user}**, open <{web}/login?code={code}> (it works once and expires \
         {expires}). Code: **{code}**\nIf you didn't ask for this, ignore this message: your password stays as it is.",
        user = clean(&link.username),
        web = data.cfg().webui_url,
        code = code.code,
        expires = relative(code.expires_at),
    );
    dm(ctx, text).await?;
    say(ctx, "I sent you a DM with a sign-in link.").await
}

/// Your linked game account.
#[poise::command(slash_command, guild_only)]
pub async fn account(ctx: Context<'_>) -> Result<(), Error> {
    let data = ctx.data();
    let Some(link) = data.api.link(user_id(ctx)).await? else {
        return say(ctx, "Your Discord account isn't linked. `/register` makes a game account, `/link` links one you have.").await;
    };
    let a = data.api.account(link.account_id).await?;
    say(ctx, describe(&data.cfg(), &a)).await
}

/// An account in a few lines.
pub fn describe(cfg: &crate::config::Config, a: &api::Account) -> String {
    let mut s = format!("**{}** (account {}), created {}", clean(&a.username), a.id, date(a.created_at));
    if a.gm_level > 0 {
        let role = cfg.gm_role_name(a.gm_level).map(|r| format!(", {r}")).unwrap_or_default();
        s += &format!("\nGM level {}{role}", a.gm_level);
    }
    match a.last_login_at {
        Some(t) => s += &format!("\nLast in game {}", relative(t)),
        None => s += "\nNever logged in to the game",
    }
    for c in &a.characters {
        s += &format!("\nCharacter **{}**, {} stars", clean(&c.name), c.stars);
    }
    if let Some(b) = &a.ban {
        let until = b.until.map(|t| format!("until {}", relative(t))).unwrap_or_else(|| "permanently".into());
        let reason = b.reason.as_deref().map(|r| format!(": {}", clean(r))).unwrap_or_default();
        s += &format!("\nBanned {until}{reason}");
    }
    s
}

/// Ask a game account to be friends: all your characters with all of theirs, once they accept.
#[poise::command(slash_command, guild_only)]
pub async fn friend(
    ctx: Context<'_>,
    #[description = "Their username (login name)"]
    #[autocomplete = "complete_username"]
    username: String,
) -> Result<(), Error> {
    let username = username.trim().to_string();
    let f = as_caller(ctx, |who| {
        let (data, username) = (ctx.data(), username.clone());
        async move { data.api.request_friend(&who, &username).await }
    })
    .await?;
    let web = ctx.data().cfg().webui_url.clone();
    if f.state == "friends" {
        say(ctx, format!("**{}** had asked you already: you are friends now, all your characters and theirs.", clean(&f.username))).await
    } else {
        say(
            ctx,
            format!(
                "Asked **{}**. Once they accept (by DM if they linked Discord, or on <{web}/friends>), all your characters \
                 and theirs are friends.",
                clean(&f.username)
            ),
        )
        .await
    }
}

/// DMs the caller; refuses with advice if their DMs are closed.
async fn dm(ctx: Context<'_>, text: String) -> Result<(), Error> {
    if ctx.author().direct_message(ctx, serenity::CreateMessage::new().content(text)).await.is_err() {
        return refuse(
            "I couldn't DM you. Allow direct messages from server members \
             (server name → Privacy Settings) and try again.",
        );
    }
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn usernames() {
        assert!(super::valid_username("Blake_1.x"));
        assert!(!super::valid_username("no spaces"));
        assert!(!super::valid_username("ab"));
    }
}
