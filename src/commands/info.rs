//! Server status and lookups (the bot's own key).

use std::collections::HashMap;

use poise::CreateReply;

use super::{clean, find_account, relative, say};
use crate::{Context, Error};

/// Whether the game server is up, and how many are playing.
#[poise::command(slash_command, guild_only)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    let s = ctx.data().api.status().await?;
    let mut text = if s.game_server_online {
        let n = s.players_online.unwrap_or(0);
        format!("The game server is **online**: {n} {} playing.", if n == 1 { "person" } else { "people" })
    } else {
        "The game server is **offline**.".to_string()
    };
    if let Some(t) = s.started_at.filter(|_| s.game_server_online) {
        text += &format!(" Up since {}.", relative(t));
    }
    if s.maintenance == Some(true) {
        text += "\nMaintenance: only staff can log in.";
        if let Some(m) = s.maintenance_message.filter(|m| !m.is_empty()) {
            text += &format!(" {}", clean(&m));
        }
    }
    if let Some(v) = s.version {
        text += &format!("\n-# LMOxide {}", clean(&v));
    }
    ctx.send(CreateReply::default().content(text)).await?;
    Ok(())
}

/// Who is playing, and where.
#[poise::command(slash_command, guild_only)]
pub async fn online(ctx: Context<'_>) -> Result<(), Error> {
    let api = &ctx.data().api;
    let players = api.online().await?;
    if players.is_empty() {
        ctx.send(CreateReply::default().content("Nobody is playing right now.")).await?;
        return Ok(());
    }
    let names: HashMap<u32, String> = api.playfields().await.unwrap_or_default().into_iter().map(|p| (p.id, p.name)).collect();
    let mut lines: Vec<String> = players
        .iter()
        .map(|p| {
            let place = p.playfield.and_then(|id| names.get(&id)).map(|n| format!(" ({})", clean(n))).unwrap_or_default();
            format!("- {}{place}", clean(&p.character_name))
        })
        .collect();
    lines.sort();
    let mut text = format!("**{} playing**\n", players.len());
    for l in lines {
        if text.len() + l.len() > 1900 {
            text += "- …";
            break;
        }
        text += &l;
        text += "\n";
    }
    ctx.send(CreateReply::default().content(text)).await?;
    Ok(())
}

/// Look up a game account by its username.
#[poise::command(slash_command, guild_only)]
pub async fn whois(ctx: Context<'_>, #[description = "Login name"] username: String) -> Result<(), Error> {
    let a = find_account(ctx, &username).await?;
    let mut text = super::account::describe(&ctx.data().cfg(), &a);
    if let Some(d) = &a.discord_id {
        text += &format!("\nDiscord: <@{d}>");
    }
    say(ctx, text).await
}
