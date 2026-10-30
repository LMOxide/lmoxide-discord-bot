//! The bot's settings file (TOML, `config.example.toml`). Secrets (the Discord token and the
//! API key) are not in it: they come from the environment or from files named here.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The LMOxide REST API, e.g. `https://lmo.example.org/api` (without `/v1`). The
    /// environment variable `LMOXIDE_API_URL` overrides it.
    pub api_url: String,
    /// The web UI, for links in DMs (`/account`, `/login`).
    pub webui_url: String,
    /// The Discord server: only its members can register or link, and commands are registered
    /// there.
    pub guild_id: u64,
    /// Discord users who may do anything with the bot (`/bot ...`, moderation without a linked
    /// account).
    #[serde(default)]
    pub admins: Vec<u64>,
    /// Where the bot keeps its database (the account keys it holds for people). Relative to the
    /// config file.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    /// Minutes between full role/GM syncs (also done at startup); 0 = only at startup and on
    /// member events.
    #[serde(default = "default_sync_interval")]
    pub sync_interval_minutes: u64,
    /// Files with the secrets, if not given in the environment.
    #[serde(default)]
    pub discord_token_file: Option<PathBuf>,
    #[serde(default)]
    pub api_key_file: Option<PathBuf>,
    pub roles: Roles,
    /// Discord roles that give a GM level in game. The highest one a member has wins; members
    /// with none of them (or who left the server) get GM 0.
    #[serde(default)]
    pub gm_roles: Vec<GmRole>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roles {
    /// Given to everyone who registers or links an account.
    pub alpha_tester: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmRole {
    /// For people reading the file and the bot's messages.
    #[serde(default)]
    pub name: String,
    pub role: u64,
    pub gm_level: u32,
}

fn default_data_dir() -> PathBuf {
    "data".into()
}

fn default_sync_interval() -> u64 {
    15
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let base = path.parent().unwrap_or(Path::new("."));
        for p in [Some(&mut cfg.data_dir), cfg.discord_token_file.as_mut(), cfg.api_key_file.as_mut()].into_iter().flatten() {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
        if let Ok(url) = std::env::var("LMOXIDE_API_URL")
            && !url.is_empty()
        {
            cfg.api_url = url;
        }
        cfg.api_url = cfg.api_url.trim_end_matches('/').to_string();
        cfg.webui_url = cfg.webui_url.trim_end_matches('/').to_string();
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !self.api_url.starts_with("http://") && !self.api_url.starts_with("https://") {
            bail!("api_url must start with http:// or https://");
        }
        let mut seen = HashSet::new();
        for r in &self.gm_roles {
            if !seen.insert(r.role) {
                bail!("gm_roles lists role {} twice", r.role);
            }
        }
        Ok(())
    }

    pub fn is_admin(&self, user: u64) -> bool {
        self.admins.contains(&user)
    }

    /// The GM level a member with these roles should have; `None` (not in the server) is 0.
    pub fn gm_level_for(&self, roles: Option<&[u64]>) -> u32 {
        let Some(roles) = roles else { return 0 };
        self.gm_roles.iter().filter(|r| roles.contains(&r.role)).map(|r| r.gm_level).max().unwrap_or(0)
    }

    /// The name of the role behind a GM level, for messages.
    pub fn gm_role_name(&self, level: u32) -> Option<&str> {
        self.gm_roles.iter().find(|r| r.gm_level == level && !r.name.is_empty()).map(|r| r.name.as_str())
    }
}

/// A secret from the environment variable `var`, the file in `<var>_FILE`, or `file`.
pub fn secret(var: &str, file: Option<&Path>) -> anyhow::Result<String> {
    if let Ok(v) = std::env::var(var)
        && !v.trim().is_empty()
    {
        return Ok(v.trim().to_string());
    }
    let file_var = format!("{var}_FILE");
    let path = std::env::var_os(&file_var).map(PathBuf::from).or_else(|| file.map(Path::to_path_buf));
    let Some(path) = path else { bail!("set {var} or {file_var}") };
    let v = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let v = v.trim();
    if v.is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_parses() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let cfg = Config::load(&dir.join("config.example.toml")).unwrap();
        assert_eq!(cfg.guild_id, 100000000000000000);
        assert_eq!(cfg.data_dir, dir.join("data"));
        assert!(cfg.is_admin(100000000000000001));
        assert!(!cfg.is_admin(1));
    }

    #[test]
    fn highest_role_wins() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let cfg = Config::load(&dir.join("config.example.toml")).unwrap();
        let admin = 100000000000000011;
        let contributor = 100000000000000012;
        let alpha = 100000000000000013;
        assert_eq!(cfg.gm_level_for(None), 0);
        assert_eq!(cfg.gm_level_for(Some(&[alpha])), 1);
        assert_eq!(cfg.gm_level_for(Some(&[alpha, contributor])), 2);
        assert_eq!(cfg.gm_level_for(Some(&[contributor, admin])), 4);
        assert_eq!(cfg.gm_role_name(2), Some("Contributor"));
        assert_eq!(cfg.gm_role_name(1), Some("Alpha Tester"));
    }
}
