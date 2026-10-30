//! Server status and lookups (the bot's own key).

use std::collections::HashMap;

use poise::{CreateReply, serenity_prelude as serenity};

use super::{clean, complete_character, complete_username, find_account, refuse, relative, say};
use crate::{Context, Error, api};

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

/// What the bot itself was built from (build.rs).
fn bot_build() -> api::Build {
    let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
    api::Build {
        version: env!("CARGO_PKG_VERSION").into(),
        commit: some(env!("BOT_GIT_COMMIT")),
        commit_time: env!("BOT_GIT_COMMIT_TIME").parse().ok(),
        built: env!("BOT_BUILD_TIME").parse().ok(),
    }
}

/// One line of `/version`: the version, the commit (linked to its repository) and when it was
/// committed and built.
fn build_line(what: &str, repo: &str, b: &api::Build) -> String {
    let mut s = format!("**{what}** {}", clean(&b.version));
    if let Some(c) = &b.commit {
        let short: String = c.chars().take(7).collect();
        if c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            s += &format!(" · [`{short}`](<https://github.com/LMOxide/{repo}/commit/{c}>)");
        } else {
            s += &format!(" · `{}`", clean(&short));
        }
    }
    if let Some(t) = b.commit_time {
        s += &format!(", committed {}", relative(t));
    }
    if let Some(t) = b.built {
        s += &format!(" · built {}", relative(t));
    }
    s
}

/// What the bot, the API, the game server and the web UIs were built from (version, commit, when).
#[poise::command(slash_command, guild_only)]
pub async fn version(ctx: Context<'_>) -> Result<(), Error> {
    let data = ctx.data();
    let cfg = data.cfg();
    let mut lines = vec![build_line("Bot", "lmoxide-discord-bot", &bot_build())];
    match data.api.version().await {
        Ok(v) => {
            match &v.game_server {
                Some(g) => {
                    let mut line = build_line("Game server", "lmoxide", g);
                    if let Some(t) = v.game_server_started_at {
                        line += &format!(" · up since {}", relative(t));
                    }
                    lines.push(line);
                }
                None => lines.push("**Game server**: offline".into()),
            }
            // (one process with the game server in the usual setup: said once)
            if v.game_server.as_ref().is_none_or(|g| g.commit != v.api.commit || g.version != v.api.version) {
                lines.push(build_line("API", "lmoxide", &v.api));
            }
        }
        Err(e) => lines.push(format!("**API**: {}", e.describe())),
    }
    for (what, repo, url) in [("Web UI", "lmoxide-webui", Some(&cfg.webui_url)), ("Dev UI", "lmoxide-dev-ui", cfg.dev_ui_url.as_ref())] {
        if let Some(url) = url
            && let Some(b) = data.api.ui_version(url).await
        {
            lines.push(build_line(what, repo, &b));
        }
    }
    ctx.send(CreateReply::default().content(lines.join("\n"))).await?;
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

/// Look up a game account by its username, its Discord account, or a character's name.
#[poise::command(slash_command, guild_only, subcommands("whois_account", "whois_discord", "whois_player"), subcommand_required)]
pub async fn whois(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// " (Discord: @member)" or "not linked".
fn discord_of(a: &crate::api::Account) -> String {
    match &a.discord_id {
        Some(d) => format!("\nDiscord: <@{d}>"),
        None => "\nNot linked to Discord".into(),
    }
}

/// A game account by its username, with the Discord account it's linked to.
#[poise::command(slash_command, rename = "account")]
async fn whois_account(
    ctx: Context<'_>,
    #[description = "Login name"]
    #[autocomplete = "complete_username"]
    username: String,
) -> Result<(), Error> {
    let a = find_account(ctx, &username).await?;
    say(ctx, super::account::describe(&ctx.data().cfg(), &a) + &discord_of(&a)).await
}

/// The game account a Discord member is linked to.
#[poise::command(slash_command, rename = "discord")]
async fn whois_discord(ctx: Context<'_>, #[description = "Discord member"] user: serenity::User) -> Result<(), Error> {
    let data = ctx.data();
    let Some(link) = data.api.link(user.id.get()).await? else {
        return say(ctx, format!("<@{}> isn't linked to a game account.", user.id)).await;
    };
    let a = data.api.account(link.account_id).await?;
    say(ctx, format!("<@{}>: {}", user.id, super::account::describe(&data.cfg(), &a))).await
}

/// Who plays a character: the game account and its Discord account.
#[poise::command(slash_command, rename = "player")]
async fn whois_player(
    ctx: Context<'_>,
    #[description = "Character name, as shown in game"]
    #[autocomplete = "complete_character"]
    name: String,
) -> Result<(), Error> {
    let data = ctx.data();
    let name = name.trim();
    let found = data.api.search_characters(name, 100).await?;
    let exact: Vec<_> = found.iter().filter(|c| c.name.eq_ignore_ascii_case(name)).collect();
    if exact.is_empty() {
        let mut text = format!("There's no character named `{}`.", clean(name));
        if !found.is_empty() {
            let names: Vec<_> = found.iter().take(10).map(|c| format!("`{}`", clean(&c.name))).collect();
            text += &format!(" Similar: {}", names.join(", "));
        }
        return refuse(text);
    }
    // a name can be on several characters (one per game mode, or different players)
    let mut text = String::new();
    let mut accounts = Vec::new();
    for c in &exact {
        if !accounts.contains(&c.account_id) {
            accounts.push(c.account_id);
        }
    }
    for id in accounts {
        let a = data.api.account(id).await?;
        let modes: Vec<_> = exact.iter().filter(|c| c.account_id == id).map(|c| c.mode.as_str()).collect();
        if !text.is_empty() {
            text += "\n\n";
        }
        text += &format!("`{}` ({}) is played by ", clean(name), modes.join(", "));
        text += &super::account::describe(&data.cfg(), &a);
        text += &discord_of(&a);
    }
    say(ctx, text).await
}
