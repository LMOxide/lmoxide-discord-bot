//! `/bot ...`: for the bot admins listed in the settings file.

use poise::serenity_prelude as serenity;

use super::{clean, refuse, say, user_id};
use crate::{Context, Error, sync};

async fn is_bot_admin(ctx: Context<'_>) -> Result<bool, Error> {
    if ctx.data().cfg().is_admin(user_id(ctx)) {
        Ok(true)
    } else {
        refuse("Only the bot's admins can do that.")
    }
}

/// Bot administration.
#[poise::command(
    slash_command,
    guild_only,
    check = "is_bot_admin",
    subcommands("sync_cmd", "reload", "info", "unlink"),
    subcommand_required
)]
pub async fn bot(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Check every linked account now: GM levels from roles, Alpha Tester role, account keys.
#[poise::command(slash_command, rename = "sync", check = "is_bot_admin")]
async fn sync_cmd(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let report = sync::full(ctx.serenity_context(), ctx.data()).await?;
    say(ctx, format!("Sync done: {}.", report.summary())).await
}

/// Re-read the settings file (roles, admins, GM levels, URLs).
#[poise::command(slash_command, check = "is_bot_admin")]
async fn reload(ctx: Context<'_>) -> Result<(), Error> {
    match ctx.data().reload() {
        Ok(cfg) => {
            say(ctx, format!("Settings reloaded: {} GM roles, {} admins. Run `/bot sync` to apply role changes.", cfg.gm_roles.len(), cfg.admins.len()))
                .await
        }
        Err(e) => refuse(format!("The settings file has a problem, kept the old settings: {e:#}")),
    }
}

/// The game account a Discord user is linked to.
#[poise::command(slash_command, check = "is_bot_admin")]
async fn info(ctx: Context<'_>, #[description = "Discord user"] user: serenity::User) -> Result<(), Error> {
    let data = ctx.data();
    let Some(link) = data.api.link(user.id.get()).await? else {
        return say(ctx, format!("<@{}> isn't linked to a game account.", user.id)).await;
    };
    let a = data.api.account(link.account_id).await?;
    let held = if data.store.key(user.id.get())?.is_some() { "the bot holds its key" } else { "the bot holds no key yet" };
    say(ctx, format!("<@{}>: {}\n-# {held}", user.id, super::account::describe(&data.cfg(), &a))).await
}

/// Unlink a Discord user from their game account (the account stays, with GM 0).
#[poise::command(slash_command, check = "is_bot_admin")]
async fn unlink(ctx: Context<'_>, #[description = "Discord user"] user: serenity::User) -> Result<(), Error> {
    let data = ctx.data();
    let id = user.id.get();
    let Some(link) = data.api.link(id).await? else {
        return refuse(format!("<@{id}> isn't linked to a game account."));
    };
    // the GM level came from Discord roles; it shouldn't outlive the link
    if link.gm_level > 0 {
        data.api.set_gm_level(id, 0).await?;
    }
    data.api.unlink(id).await?;
    data.store.remove_key(id)?;
    tracing::info!(user = %link.username, discord = id, by = user_id(ctx), "unlinked");
    say(ctx, format!("Unlinked <@{id}> from `{}`.", clean(&link.username))).await
}
