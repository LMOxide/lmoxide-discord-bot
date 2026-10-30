//! The LMOxide Discord bot: registers game accounts for members of the Discord server, links
//! existing ones, sends sign-in codes for password resets, keeps GM levels in step with Discord
//! roles, and runs status and moderation commands. It talks only to the LMOxide REST API.

mod api;
mod commands;
mod config;
mod store;
mod sync;

#[cfg(test)]
mod live_tests;

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock, atomic::AtomicBool},
};

use anyhow::Context as _;
use clap::Parser;
use poise::serenity_prelude as serenity;
use tracing_subscriber::EnvFilter;

use crate::{config::Config, store::Store};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// The settings file (see config.example.toml).
    #[arg(long, env = "LMOXIDE_BOT_CONFIG", default_value = "config.toml")]
    config: PathBuf,
}

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;

/// Shared by the commands, the event handler and background tasks (cheap to clone).
#[derive(Clone)]
pub struct Data(Arc<Inner>);

impl std::ops::Deref for Data {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

pub struct Inner {
    cfg: RwLock<Arc<Config>>,
    pub config_path: PathBuf,
    pub api: api::Client,
    pub store: Store,
    /// One full sync at a time.
    pub sync_lock: tokio::sync::Mutex<()>,
    /// Discord users with a `/link` code the bot is waiting on.
    pub pending_links: Mutex<HashSet<u64>>,
    /// The periodic sync runs (started at the first Ready).
    pub sync_started: AtomicBool,
}

impl Inner {
    pub fn cfg(&self) -> Arc<Config> {
        self.cfg.read().unwrap().clone()
    }

    /// Re-reads the settings file. The guild, data directory and secrets stay as they were
    /// until a restart.
    pub fn reload(&self) -> anyhow::Result<Arc<Config>> {
        let new = Config::load(&self.config_path)?;
        let mut cur = self.cfg.write().unwrap();
        if new.guild_id != cur.guild_id || new.api_url != cur.api_url {
            anyhow::bail!("guild_id and api_url can only change with a restart");
        }
        *cur = Arc::new(new);
        Ok(cur.clone())
    }

    pub fn guild(&self) -> serenity::GuildId {
        serenity::GuildId::new(self.cfg().guild_id)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,serenity=warn".into()))
        .init();
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    let token = config::secret("DISCORD_TOKEN", cfg.discord_token_file.as_deref())?;
    let key = config::secret("LMOXIDE_API_KEY", cfg.api_key_file.as_deref())?;
    let store = Store::open(&cfg.data_dir)?;
    let api = api::Client::new(&cfg.api_url, key);
    match api.config().await {
        Ok(c) => tracing::info!(api = %cfg.api_url, registration = ?c.registration, "API reachable"),
        Err(e) => tracing::warn!(api = %cfg.api_url, "API not reachable yet: {e}"),
    }

    let data = Data(Arc::new(Inner {
        cfg: RwLock::new(Arc::new(cfg)),
        config_path: cli.config,
        api,
        store,
        sync_lock: Default::default(),
        pending_links: Default::default(),
        sync_started: AtomicBool::new(false),
    }));
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: commands::all(),
            on_error: |e| Box::pin(commands::on_error(e)),
            event_handler: |fw, event| Box::pin(sync::on_event(fw, event)),
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                tracing::info!(user = %ready.user.name, "connected to Discord");
                poise::builtins::register_in_guild(ctx, &framework.options().commands, data.guild())
                    .await
                    .context("registering the commands in the server")?;
                Ok(data)
            })
        })
        .build();

    // GUILD_MEMBERS (privileged): role changes, joins and leaves, and listing members to sync
    let intents = serenity::GatewayIntents::GUILDS | serenity::GatewayIntents::GUILD_MEMBERS;
    let mut client = serenity::ClientBuilder::new(token, intents).framework(framework).await?;
    let shards = client.shard_manager.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("stopping");
        shards.shutdown_all().await;
    });
    client.start().await?;
    Ok(())
}
