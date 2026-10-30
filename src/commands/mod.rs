//! Slash commands, registered in the configured server. Answers are ephemeral (only the person
//! who ran the command sees them), except `/status` and `/online`; codes go in DMs.

mod account;
mod admin;
mod info;
mod moderation;

use poise::{CreateReply, serenity_prelude as serenity};

use crate::{Context, Data, Error, api};

pub fn all() -> Vec<poise::Command<Data, Error>> {
    vec![
        account::register(),
        account::link(),
        account::reset_password(),
        account::account(),
        info::status(),
        info::online(),
        info::whois(),
        info::version(),
        account::friend(),
        moderation::kick(),
        moderation::ban(),
        moderation::unban(),
        moderation::announce(),
        admin::bot(),
    ]
}

/// An error whose message is meant for the person who ran the command.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refusal(pub String);

pub fn refuse<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Refusal(msg.into()).into())
}

pub async fn on_error(e: poise::FrameworkError<'_, Data, Error>) {
    let poise::FrameworkError::Command { error, ctx, .. } = e else {
        if let Err(e) = poise::builtins::on_error(e).await {
            tracing::warn!("error handler: {e}");
        }
        return;
    };
    let text = if let Some(r) = error.downcast_ref::<Refusal>() {
        r.0.clone()
    } else if let Some(a) = error.downcast_ref::<api::ApiError>() {
        if a.code().is_none() || a.status().is_some_and(|s| s.is_server_error()) {
            tracing::warn!(command = %ctx.command().qualified_name, "{error:#}");
        }
        a.describe()
    } else {
        tracing::error!(command = %ctx.command().qualified_name, "{error:#}");
        "Something went wrong; the bot's log has the details.".into()
    };
    if let Err(e) = say(ctx, text).await {
        tracing::warn!("couldn't answer: {e}");
    }
}

/// An ephemeral answer.
pub async fn say(ctx: Context<'_>, text: impl Into<String>) -> Result<(), Error> {
    ctx.send(CreateReply::default().content(text).ephemeral(true)).await?;
    Ok(())
}

pub fn user_id(ctx: Context<'_>) -> u64 {
    ctx.author().id.get()
}

/// The caller's roles in the configured server; refuses people who aren't in it.
pub async fn member_roles(ctx: Context<'_>) -> Result<Vec<u64>, Error> {
    let data = ctx.data();
    match data.guild().member(ctx, ctx.author().id).await {
        Ok(m) => Ok(m.roles.iter().map(|r| r.get()).collect()),
        Err(serenity::Error::Http(e)) if e.status_code() == Some(serenity::StatusCode::NOT_FOUND) => {
            refuse("You need to be a member of the LMOxide Discord server for that.")
        }
        Err(e) => Err(e.into()),
    }
}

/// `discord:<name> (<id>)` for the audit log (`X-On-Behalf-Of`).
pub fn behalf(ctx: Context<'_>) -> String {
    let mut name = ctx.author().name.clone();
    name.truncate(60);
    format!("discord:{name} ({})", ctx.author().id)
}

/// The credentials to act with for the caller: their account key (made if the bot doesn't hold
/// one yet), or for bot admins without a linked account the bot's own key.
pub async fn actor(ctx: Context<'_>) -> Result<api::As, Error> {
    let data = ctx.data();
    let me = user_id(ctx);
    if let Some(k) = data.store.key(me)? {
        return Ok(api::As::Account(k.secret));
    }
    match data.api.link(me).await? {
        Some(link) => {
            let key = data.api.account_key(me).await?;
            data.store.set_key(me, link.account_id, &key.secret)?;
            Ok(api::As::Account(key.secret))
        }
        None if data.cfg().is_admin(me) => Ok(api::As::Bot { behalf: Some(behalf(ctx)) }),
        None => refuse("Link your game account first: `/register` for a new one, `/link` for one you have."),
    }
}

/// Runs `f` as the caller; if their account key was revoked meanwhile, makes a new one and
/// tries once more.
pub async fn as_caller<T, F, Fut>(ctx: Context<'_>, f: F) -> Result<T, Error>
where
    F: Fn(api::As) -> Fut,
    Fut: Future<Output = api::Result<T>>,
{
    let who = actor(ctx).await?;
    let is_account = matches!(who, api::As::Account(_));
    match f(who).await {
        Err(e) if is_account && e.code() == Some("unauthorized") => {
            let data = ctx.data();
            let me = user_id(ctx);
            data.store.remove_key(me)?;
            Ok(f(actor(ctx).await?).await?)
        }
        r => Ok(r?),
    }
}

/// The account with this username, looked up with the bot's key.
pub async fn find_account(ctx: Context<'_>, username: &str) -> Result<api::Account, Error> {
    match ctx.data().api.account_by_name(username).await? {
        Some(a) => Ok(a),
        None => refuse(format!("There's no account named `{}`.", clean(username))),
    }
}

/// Discord shows at most 25 suggestions.
const SUGGESTIONS: u32 = 25;

/// Autocomplete for usernames (login names).
pub async fn complete_username(ctx: Context<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let found = ctx.data().api.search_accounts(partial.trim(), SUGGESTIONS).await.unwrap_or_default();
    let choices = found.into_iter().map(|a| serenity::AutocompleteChoice::from(a.username)).collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

/// Autocomplete for character (in-game) names, labelled with whose they are.
pub async fn complete_character(ctx: Context<'_>, partial: &str) -> serenity::CreateAutocompleteResponse {
    let found = ctx.data().api.search_characters(partial.trim(), SUGGESTIONS).await.unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let choices = found
        .into_iter()
        .filter(|c| seen.insert(c.name.to_lowercase()))
        .map(|c| {
            let mut label = format!("{} ({}, {})", c.name, c.username, c.mode);
            label.truncate(100);
            serenity::AutocompleteChoice::new(label, c.name)
        })
        .collect();
    serenity::CreateAutocompleteResponse::new().set_choices(choices)
}

/// Text that can't format or ping in a Discord message.
pub fn clean(s: &str) -> String {
    s.replace('`', "'").replace('@', "@\u{200b}")
}

/// A Discord timestamp (`<t:..:R>` "in 10 minutes").
pub fn relative(unix: i64) -> String {
    format!("<t:{unix}:R>")
}

pub fn date(unix: i64) -> String {
    format!("<t:{unix}:D>")
}
