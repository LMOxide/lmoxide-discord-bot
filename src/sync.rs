//! Keeping linked accounts in step with Discord: the GM level follows the member's roles
//! (`gm_roles`), linked members get the Alpha Tester role, and the bot holds an account key for
//! each linked member.
//!
//! A full sync runs at startup (things may have changed while the bot or the API was down),
//! every `sync_interval_minutes`, and on `/bot sync`. Role changes, joins and leaves sync that
//! one member at once. Members who leave keep their account with GM 0.

use std::{collections::HashMap, time::Duration};

use anyhow::Context as _;
use poise::serenity_prelude as serenity;

use crate::{Data, Error, api, config::Config};

/// What a sync changed.
#[derive(Debug, Default)]
pub struct Report {
    pub links: usize,
    /// (username, from, to)
    pub gm_changes: Vec<(String, u32, u32)>,
    pub roles_given: usize,
    pub keys_made: usize,
    pub keys_dropped: usize,
    pub errors: Vec<String>,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut s = format!("{} linked accounts checked", self.links);
        if !self.gm_changes.is_empty() {
            let list: Vec<_> = self.gm_changes.iter().map(|(u, a, b)| format!("{u} GM {a}→{b}")).collect();
            s += &format!("; GM levels: {}", list.join(", "));
        }
        if self.roles_given > 0 {
            s += &format!("; Alpha Tester given to {}", self.roles_given);
        }
        if self.keys_made > 0 {
            s += &format!("; {} account keys made", self.keys_made);
        }
        if self.keys_dropped > 0 {
            s += &format!("; {} keys of unlinked accounts dropped", self.keys_dropped);
        }
        if !self.errors.is_empty() {
            s += &format!("; {} errors: {}", self.errors.len(), self.errors.join("; "));
        }
        s
    }
}

/// Pause between changes in a full sync, to stay far below the API's rate limit.
const WRITE_PAUSE: Duration = Duration::from_millis(150);

fn role_ids(m: &serenity::Member) -> Vec<u64> {
    m.roles.iter().map(|r| r.get()).collect()
}

/// Every member of the server with their roles.
async fn all_members(ctx: &serenity::Context, guild: serenity::GuildId) -> anyhow::Result<HashMap<u64, Vec<u64>>> {
    let mut out = HashMap::new();
    let mut after = None;
    loop {
        let page = guild.members(&ctx.http, Some(1000), after).await.context("listing the server's members")?;
        let Some(last) = page.last() else { break };
        after = Some(last.user.id);
        let n = page.len();
        out.extend(page.iter().map(|m| (m.user.id.get(), role_ids(m))));
        if n < 1000 {
            break;
        }
    }
    Ok(out)
}

/// Brings one linked account in line. `roles` is `None` when the person isn't in the server.
async fn sync_link(
    ctx: &serenity::Context,
    data: &Data,
    cfg: &Config,
    link: &api::DiscordLink,
    roles: Option<&[u64]>,
    report: &mut Report,
) -> anyhow::Result<()> {
    let discord_id: u64 = link.discord_id.parse().context("discord id")?;
    let want = cfg.gm_level_for(roles);
    if link.gm_level != want {
        data.api.set_gm_level(discord_id, want).await?;
        tracing::info!(user = %link.username, from = link.gm_level, to = want, "GM level synced");
        report.gm_changes.push((link.username.clone(), link.gm_level, want));
    }
    let Some(roles) = roles else { return Ok(()) };
    if let Some(alpha) = cfg.roles.alpha_tester
        && !roles.contains(&alpha)
    {
        ctx.http
            .add_member_role(
                serenity::GuildId::new(cfg.guild_id),
                serenity::UserId::new(discord_id),
                serenity::RoleId::new(alpha),
                Some("linked to an LMOxide account"),
            )
            .await
            .context("giving the Alpha Tester role")?;
        report.roles_given += 1;
    }
    let held = data.store.key(discord_id)?;
    if held.is_none_or(|k| k.account_id != link.account_id) {
        let key = data.api.account_key(discord_id).await?;
        data.store.set_key(discord_id, link.account_id, &key.secret)?;
        report.keys_made += 1;
    }
    Ok(())
}

/// Checks every linked account.
pub async fn full(ctx: &serenity::Context, data: &Data) -> anyhow::Result<Report> {
    let _one = data.sync_lock.lock().await;
    let cfg = data.cfg();
    let links = data.api.links().await.context("listing linked accounts")?;
    let members = all_members(ctx, serenity::GuildId::new(cfg.guild_id)).await?;
    let mut report = Report { links: links.len(), ..Default::default() };
    for link in &links {
        let roles = link.discord_id.parse().ok().and_then(|id: u64| members.get(&id));
        let before = (report.gm_changes.len(), report.roles_given, report.keys_made);
        if let Err(e) = sync_link(ctx, data, &cfg, link, roles.map(Vec::as_slice), &mut report).await {
            tracing::warn!(user = %link.username, "sync: {e:#}");
            report.errors.push(format!("{}: {e:#}", link.username));
        }
        if before != (report.gm_changes.len(), report.roles_given, report.keys_made) {
            tokio::time::sleep(WRITE_PAUSE).await;
        }
    }
    let linked: std::collections::HashSet<u64> = links.iter().filter_map(|l| l.discord_id.parse().ok()).collect();
    for holder in data.store.holders()? {
        if !linked.contains(&holder) {
            data.store.remove_key(holder)?;
            report.keys_dropped += 1;
        }
    }
    tracing::info!("sync: {}", report.summary());
    Ok(report)
}

/// Syncs one Discord user, if linked. `roles` is `None` when they aren't in the server.
pub async fn member(ctx: &serenity::Context, data: &Data, user: u64, roles: Option<&[u64]>) -> anyhow::Result<Report> {
    let mut report = Report::default();
    if let Some(link) = data.api.link(user).await? {
        report.links = 1;
        sync_link(ctx, data, &data.cfg(), &link, roles, &mut report).await?;
    }
    Ok(report)
}

/// [`member`], looking up the member's roles first.
pub async fn member_by_id(ctx: &serenity::Context, data: &Data, user: u64) -> anyhow::Result<Report> {
    let roles = match data.guild().member(ctx, serenity::UserId::new(user)).await {
        Ok(m) => Some(role_ids(&m)),
        Err(serenity::Error::Http(e)) if e.status_code() == Some(serenity::StatusCode::NOT_FOUND) => None,
        Err(e) => return Err(e.into()),
    };
    member(ctx, data, user, roles.as_deref()).await
}

pub async fn on_event(fw: poise::FrameworkContext<'_, Data, Error>, event: &serenity::FullEvent) -> Result<(), Error> {
    let ctx = fw.serenity_context;
    let data = fw.user_data;
    let guild = data.guild();
    let one = |user: u64, roles: Option<Vec<u64>>| {
        let (ctx, data) = (ctx.clone(), data.clone());
        tokio::spawn(async move {
            if let Err(e) = member(&ctx, &data, user, roles.as_deref()).await {
                tracing::warn!(user, "member sync: {e:#}");
            }
        });
    };
    match event {
        serenity::FullEvent::Ready { .. } if !data.sync_started.swap(true, std::sync::atomic::Ordering::SeqCst) => {
            start_periodic(ctx.clone(), data.clone());
            crate::friends::start(ctx.clone(), data.clone());
        }
        serenity::FullEvent::InteractionCreate { interaction: serenity::Interaction::Component(c) } if crate::friends::is_ours(c) => {
            let (ctx, data, c) = (ctx.clone(), data.clone(), c.clone());
            tokio::spawn(async move {
                if let Err(e) = crate::friends::on_button(&ctx, &data, &c).await {
                    tracing::warn!("friend request button: {e:#}");
                }
            });
        }
        serenity::FullEvent::GuildMemberAddition { new_member } if new_member.guild_id == guild => {
            one(new_member.user.id.get(), Some(role_ids(new_member)));
        }
        serenity::FullEvent::GuildMemberUpdate { event, .. } if event.guild_id == guild => {
            one(event.user.id.get(), Some(event.roles.iter().map(|r| r.get()).collect()));
        }
        serenity::FullEvent::GuildMemberRemoval { guild_id, user, .. } if *guild_id == guild => {
            one(user.id.get(), None);
        }
        _ => {}
    }
    Ok(())
}

/// A full sync now, then every `sync_interval_minutes`.
fn start_periodic(ctx: serenity::Context, data: Data) {
    tokio::spawn(async move {
        loop {
            // a failed sync (the API down?) is tried again after a minute
            let minutes = match full(&ctx, &data).await {
                Ok(_) => data.cfg().sync_interval_minutes.max(1),
                Err(e) => {
                    tracing::warn!("sync failed: {e:#}");
                    1
                }
            };
            tokio::time::sleep(Duration::from_secs(60 * minutes)).await;
            // 0: no periodic sync; look again later in case /bot reload turns it on
            while data.cfg().sync_interval_minutes == 0 {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    });
}
