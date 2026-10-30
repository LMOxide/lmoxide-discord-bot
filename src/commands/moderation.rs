//! Moderation, done with the caller's own account key: the API decides from their GM level
//! (which follows their Discord roles) whether they may, and the audit log names them.

use super::{as_caller, clean, complete_username, find_account, relative, say};
use crate::{Context, Error};

/// Disconnect a player from the game.
#[poise::command(slash_command, guild_only)]
pub async fn kick(
    ctx: Context<'_>,
    #[description = "Login name"]
    #[autocomplete = "complete_username"]
    username: String,
    #[description = "Shown in the audit log"]
    #[max_length = 200]
    reason: Option<String>,
) -> Result<(), Error> {
    let a = find_account(ctx, &username).await?;
    let api = ctx.data().api.clone();
    let r = as_caller(ctx, |who| {
        let (api, reason) = (api.clone(), reason.clone());
        async move { api.kick(&who, a.id, reason.as_deref()).await }
    })
    .await?;
    let name = clean(&a.username);
    say(ctx, if r.affected > 0 { format!("Kicked `{name}`.") } else { format!("`{name}` isn't online.") }).await
}

/// Ban an account (and kick it). Without days the ban is permanent.
#[poise::command(slash_command, guild_only)]
pub async fn ban(
    ctx: Context<'_>,
    #[description = "Login name"]
    #[autocomplete = "complete_username"]
    username: String,
    #[description = "Length in days; leave empty for a permanent ban"]
    #[min = 1]
    #[max = 3650]
    days: Option<u32>,
    #[description = "Shown to the player and in the audit log"]
    #[max_length = 200]
    reason: Option<String>,
) -> Result<(), Error> {
    let a = find_account(ctx, &username).await?;
    let api = ctx.data().api.clone();
    let after = as_caller(ctx, |who| {
        let (api, reason) = (api.clone(), reason.clone());
        async move { api.ban(&who, a.id, days, reason.as_deref()).await }
    })
    .await?;
    let until = after.ban.and_then(|b| b.until).map(|t| format!(" until {}", relative(t))).unwrap_or_else(|| " permanently".into());
    say(ctx, format!("Banned `{}`{until}.", clean(&a.username))).await
}

/// Lift an account's ban.
#[poise::command(slash_command, guild_only)]
pub async fn unban(
    ctx: Context<'_>,
    #[description = "Login name"]
    #[autocomplete = "complete_username"]
    username: String,
) -> Result<(), Error> {
    let a = find_account(ctx, &username).await?;
    let api = ctx.data().api.clone();
    as_caller(ctx, |who| {
        let api = api.clone();
        async move { api.unban(&who, a.id).await }
    })
    .await?;
    say(ctx, format!("Unbanned `{}`.", clean(&a.username))).await
}

/// Say something in game chat to everyone online (from "Server").
#[poise::command(slash_command, guild_only)]
pub async fn announce(
    ctx: Context<'_>,
    #[description = "The message"]
    #[min_length = 1]
    #[max_length = 500]
    message: String,
) -> Result<(), Error> {
    let api = ctx.data().api.clone();
    let r = as_caller(ctx, |who| {
        let (api, message) = (api.clone(), message.clone());
        async move { api.broadcast(&who, &message).await }
    })
    .await?;
    say(ctx, format!("Sent to {} {} in game.", r.affected, if r.affected == 1 { "player" } else { "players" })).await
}
