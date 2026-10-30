//! Account friend requests by DM: someone asks another account to be friends (the web UI or
//! `/friend`); if that account is linked to Discord, the bot DMs its member the request with
//! Accept and Reject buttons, and the buttons answer as that member's account (its key). The API
//! has no events, so the bot asks it every few seconds for requests it hasn't sent yet
//! (`GET /v1/discord/friend-requests`) and marks each one sent.

use std::time::Duration;

use anyhow::Context as _;
use poise::serenity_prelude as serenity;

use crate::{Data, api, commands::clean};

/// How often the bot asks the API for new requests.
const POLL: Duration = Duration::from_secs(10);
/// The buttons' custom ids: `friend:accept:<requester>:<target>`, `friend:reject:...`.
const PREFIX: &str = "friend:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Accept,
    Reject,
}

fn custom_id(answer: Answer, requester: u32, target: u32) -> String {
    let a = if answer == Answer::Accept { "accept" } else { "reject" };
    format!("{PREFIX}{a}:{requester}:{target}")
}

fn parse(id: &str) -> Option<(Answer, u32, u32)> {
    let mut parts = id.strip_prefix(PREFIX)?.split(':');
    let answer = match parts.next()? {
        "accept" => Answer::Accept,
        "reject" => Answer::Reject,
        _ => return None,
    };
    let requester = parts.next()?.parse().ok()?;
    let target = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((answer, requester, target))
}

/// Whether a component interaction is one of these buttons.
pub fn is_ours(c: &serenity::ComponentInteraction) -> bool {
    c.data.custom_id.starts_with(PREFIX)
}

/// Polls for new requests from now on (started at the first Ready).
pub fn start(ctx: serenity::Context, data: Data) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = send_new(&ctx, &data).await {
                tracing::debug!("friend requests: {e:#}");
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

/// DMs every request not sent yet, then marks it sent (also when the DM failed: the member's DMs
/// are closed, and the web UI shows the request anyway).
async fn send_new(ctx: &serenity::Context, data: &Data) -> anyhow::Result<()> {
    for r in data.api.friend_requests().await? {
        let user: u64 = r.discord_id.parse().context("discord id")?;
        let text = format!(
            "**{}** asks your account **{}** to be friends: all your characters and all of theirs become friends in game, \
             characters you make later too.\nYou can also answer on <{}/friends>.",
            clean(&r.requester_username),
            clean(&r.target_username),
            data.cfg().webui_url,
        );
        let buttons = serenity::CreateActionRow::Buttons(vec![
            serenity::CreateButton::new(custom_id(Answer::Accept, r.requester_id, r.target_id))
                .label("Accept")
                .style(serenity::ButtonStyle::Success),
            serenity::CreateButton::new(custom_id(Answer::Reject, r.requester_id, r.target_id))
                .label("Reject")
                .style(serenity::ButtonStyle::Danger),
        ]);
        let msg = serenity::CreateMessage::new().content(text).components(vec![buttons]);
        match serenity::UserId::new(user).direct_message(ctx, msg).await {
            Ok(_) => tracing::info!(from = %r.requester_username, to = %r.target_username, "friend request sent by DM"),
            Err(e) => tracing::info!(to = %r.target_username, discord = user, "friend request: couldn't DM: {e}"),
        }
        data.api.friend_request_sent(r.requester_id, r.target_id).await?;
    }
    Ok(())
}

/// The account key the bot holds for a linked member (made if it has none); none if they
/// aren't linked.
async fn key_of(data: &Data, user: u64) -> anyhow::Result<Option<crate::store::HeldKey>> {
    if let Some(k) = data.store.key(user)? {
        return Ok(Some(k));
    }
    let Some(link) = data.api.link(user).await? else { return Ok(None) };
    let key = data.api.account_key(user).await?;
    data.store.set_key(user, link.account_id, &key.secret)?;
    data.store.key(user)
}

/// Accept or Reject pressed: answers as the member's account, then replaces the buttons with
/// what happened.
pub async fn on_button(ctx: &serenity::Context, data: &Data, c: &serenity::ComponentInteraction) -> anyhow::Result<()> {
    let Some((answer, requester, target)) = parse(&c.data.custom_id) else { return Ok(()) };
    let user = c.user.id.get();
    let text = match answer_as(data, user, answer, requester, target).await {
        Ok(t) => t,
        Err(e) => match e.downcast_ref::<api::ApiError>() {
            Some(a) if a.is_not_found() => "That request isn't waiting any more (answered or withdrawn).".into(),
            Some(a) => a.describe(),
            None => {
                tracing::warn!(discord = user, "friend request answer: {e:#}");
                "Something went wrong; the bot's log has the details.".into()
            }
        },
    };
    let done = serenity::CreateInteractionResponseMessage::new().content(text).components(vec![]);
    c.create_response(ctx, serenity::CreateInteractionResponse::UpdateMessage(done)).await?;
    Ok(())
}

async fn answer_as(data: &Data, user: u64, answer: Answer, requester: u32, target: u32) -> anyhow::Result<String> {
    let mut tried_new_key = false;
    loop {
        let Some(key) = key_of(data, user).await? else {
            return Ok("Your Discord account isn't linked to that game account any more.".into());
        };
        if key.account_id != target {
            return Ok("This request is for a game account your Discord account isn't linked to any more.".into());
        }
        let who = api::As::Account(key.secret);
        let r = match answer {
            Answer::Accept => data.api.accept_friend(&who, requester).await.map(|f| {
                format!("You and **{}** are friends now: all your characters and theirs.", clean(&f.username))
            }),
            Answer::Reject => data.api.reject_friend(&who, requester).await.map(|()| "Turned down.".to_string()),
        };
        match r {
            // the key was revoked meanwhile: make a new one, once
            Err(e) if e.code() == Some("unauthorized") && !tried_new_key => {
                data.store.remove_key(user)?;
                tried_new_key = true;
            }
            r => return Ok(r?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_ids_round_trip() {
        let id = custom_id(Answer::Accept, 16000001, 20000002);
        assert_eq!(id, "friend:accept:16000001:20000002");
        assert_eq!(parse(&id), Some((Answer::Accept, 16000001, 20000002)));
        assert_eq!(parse(&custom_id(Answer::Reject, 1, 2)), Some((Answer::Reject, 1, 2)));
        assert_eq!(parse("friend:accept:1"), None);
        assert_eq!(parse("friend:accept:1:2:3"), None);
        assert_eq!(parse("other:accept:1:2"), None);
        assert!(id.len() <= 100, "Discord's custom id limit");
    }
}
