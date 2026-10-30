//! The bot's own database: the account key it holds for each linked Discord user. The keys
//! are secrets (they act as the account), so the file is readable only by the bot's user.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Context as _;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldKey {
    pub account_id: u32,
    pub secret: String,
}

impl Store {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("bot.db");
        let conn = Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS account_keys (
                 discord_id  INTEGER PRIMARY KEY,
                 account_id  INTEGER NOT NULL,
                 secret      TEXT NOT NULL,
                 created_at  INTEGER NOT NULL
             );",
        )?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    pub fn key(&self, discord_id: u64) -> anyhow::Result<Option<HeldKey>> {
        Ok(self
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT account_id, secret FROM account_keys WHERE discord_id = ?1", [discord_id as i64], |r| {
                Ok(HeldKey { account_id: r.get(0)?, secret: r.get(1)? })
            })
            .optional()?)
    }

    pub fn set_key(&self, discord_id: u64, account_id: u32, secret: &str) -> anyhow::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO account_keys (discord_id, account_id, secret, created_at) VALUES (?1, ?2, ?3, unixepoch())
             ON CONFLICT (discord_id) DO UPDATE SET account_id = ?2, secret = ?3, created_at = unixepoch()",
            params![discord_id as i64, account_id, secret],
        )?;
        Ok(())
    }

    pub fn remove_key(&self, discord_id: u64) -> anyhow::Result<()> {
        self.conn.lock().unwrap().execute("DELETE FROM account_keys WHERE discord_id = ?1", [discord_id as i64])?;
        Ok(())
    }

    /// Discord users the bot holds a key for.
    pub fn holders(&self) -> anyhow::Result<Vec<u64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT discord_id FROM account_keys")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.map(|r| r.map(|v| v as u64)).collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        let s = Store::in_memory();
        let big = 1563439888093937844u64;
        assert_eq!(s.key(big).unwrap(), None);
        s.set_key(big, 7, "lmox_a").unwrap();
        s.set_key(big, 7, "lmox_b").unwrap();
        assert_eq!(s.key(big).unwrap(), Some(HeldKey { account_id: 7, secret: "lmox_b".into() }));
        assert_eq!(s.holders().unwrap(), vec![big]);
        s.remove_key(big).unwrap();
        assert!(s.holders().unwrap().is_empty());
    }
}
