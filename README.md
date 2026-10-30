# LMOxide Discord bot

The Discord bot for an [LMOxide](https://github.com/LMOxide/lmoxide) server (LEGO Minifigures
Online). Members of the Discord server get their game accounts through it, and their Discord
roles decide their GM level in game.

| Command | Who | What it does |
|---|---|---|
| `/register` | members | a form (username, password) that creates a game account linked to their Discord account, and gives them the Alpha Tester role |
| `/link` | members | DMs a code; entered in the web UI while signed in, it links an existing game account |
| `/reset-password` | linked members | DMs a one-time sign-in link to the web UI, which asks for a new password |
| `/account` | linked members | their game account: GM level, characters, last login, ban |
| `/status`, `/online` | everyone | whether the game server is up; who is playing and where (answered in the channel) |
| `/whois account <username>`, `/whois discord <member>` | everyone | a game account by login name (with its Discord account), or the game account a Discord member is linked to |
| `/kick`, `/ban`, `/unban`, `/announce` | GMs (by role) | moderation, done as their own game account |
| `/bot sync`, `/bot reload`, `/bot info`, `/bot unlink` | bot admins | sync now, re-read the settings, look up or unlink a member |

Answers are visible only to the person who ran the command, except `/status` and `/online`.

## How it works

The bot talks only to the LMOxide REST API (`lmoxide-api`), with two kinds of keys:

- **its own key**, scope `discord`: lookups, registering accounts, linking, one-time codes,
  setting GM levels of linked accounts. It can't touch accounts that aren't linked to Discord,
  except for moderation by bot admins.
- **an account key for each linked member**, made by the API when they register or link and kept
  in the bot's database (`data/bot.db`). Commands such as `/kick` run with the caller's key, so
  the API decides from their GM level whether they may, and the audit log names their account.

**Roles → GM levels.** `gm_roles` in the settings maps Discord roles to GM levels (Admin → 2,
Contributor → 1). The highest one wins. Members with none of them, or who left the server, get
GM 0 and keep their account. Discord decides: a GM level set elsewhere on a linked account is
overwritten at the next sync. The bot syncs at startup (things may have changed while it was
offline), every `sync_interval_minutes`, on `/bot sync`, and at once when a member's roles change
or they join or leave. The sync also gives linked members the Alpha Tester role and makes missing
account keys.

**Registration modes** (the API's `--registration`): `discord` makes the bot the only way to get
an account, so every player is a member of the Discord server. `open` also allows the web UI's
register page; `closed` allows neither.

**Codes.** `/link` and `/reset-password` codes are made by the API, last 10 minutes, work once,
and go only to the member's DMs. A link completes in the web UI. The bot checks every few seconds
until the code expires, then sets the GM level and roles and confirms by DM.

## Setup

See [docs/DEPLOYING.md](docs/DEPLOYING.md) for creating the Discord application, the API key and
running it in Docker. In short:

```sh
cp config.example.toml config.toml        # server, roles, admins, URLs
export DISCORD_TOKEN=...                   # or DISCORD_TOKEN_FILE
export LMOXIDE_API_KEY=lmox_...            # lmoxide-api key create discord-bot --scope discord
cargo run --release
```

## Development

[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): a local API to develop against, tests, and the API
contract. API changes the bot needed are recorded in [docs/API-PROPOSALS.md](docs/API-PROPOSALS.md).
