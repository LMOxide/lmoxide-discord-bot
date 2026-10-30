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
| `/version` | everyone | what the bot, the game server (and API) and the web UI and dev UI are on: version, commit (linked), when it was committed and built (answered in the channel) |
| `/friend <username>` | linked members | asks that game account to be friends (account friends: all your characters with all of theirs, once they accept) |
| `/whois account <username>`, `/whois discord <member>`, `/whois player <character>` | everyone | a game account by login name, by Discord member, or by character name, with the Discord account it's linked to; usernames and character names autocomplete |
| `/kick`, `/ban`, `/unban`, `/announce` | GMs (by role) | moderation, done as their own game account |
| `/bot sync`, `/bot reload`, `/bot info`, `/bot unlink` | bot admins | sync now, re-read the settings, look up or unlink a member |

Answers are visible only to the person who ran the command, except `/status`, `/online` and `/version`.

## How it works

The bot talks only to the LMOxide REST API (`lmoxide-api`), with two kinds of keys:

- **its own key**, scope `discord`: lookups, registering accounts, linking, one-time codes,
  setting GM levels of linked accounts. It can't touch accounts that aren't linked to Discord,
  except for moderation by bot admins.
- **an account key for each linked member**, made by the API when they register or link and kept
  in the bot's database (`data/bot.db`). Commands such as `/kick` run with the caller's key, so
  the API decides from their GM level whether they may, and the audit log names their account.

**Roles → GM levels.** `gm_roles` in the settings maps Discord roles to GM levels (Admin → 4,
Contributor → 2, Alpha Tester → 1; LMOxide's `docs/SECURITY.md` "Access levels": alpha testers
play and use the testers' commands from the dev UI, contributors also get the world editor and
the client's GM tools). The highest one wins. Members with none of them, or who left the server, get
GM 0 and keep their account. Discord decides: a GM level set elsewhere on a linked account is
overwritten at the next sync. The bot syncs at startup (things may have changed while it was
offline), every `sync_interval_minutes`, on `/bot sync`, and at once when a member's roles change
or they join or leave. The sync also gives linked members the Alpha Tester role and makes missing
account keys.

**Registration modes** (the API's `--registration`): `discord` makes the bot the only way to get
an account, so every player is a member of the Discord server. `open` also allows the web UI's
register page; `closed` allows neither.

**Friend requests.** When someone asks an account linked to Discord to be friends (the web UI's
Friends page or `/friend`), the bot DMs its member the request with **Accept** and **Reject**
buttons; a button answers with the member's account key. The API has no events: the bot asks
`GET /v1/discord/friend-requests` every 10 seconds and marks each request sent (also when the
member's DMs are closed; the web UI shows it anyway).

**Versions.** `/version` reads `GET /v1/version` and the UIs' `version.json` (`webui_url`, and
`dev_ui_url` if set). The bot's own commit is built in (build.rs); Docker builds have no `.git`,
so the build arguments `GIT_COMMIT` and `GIT_COMMIT_TIME` give it (the release workflow does).

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

## Install without Rust

Every version tag publishes the bot:

| | |
|---|---|
| Image | `ghcr.io/lmoxide/lmoxide-discord-bot` (`latest` or a version) |
| Binary | `lmoxide-discord-bot-<version>-x86_64-linux.tar.gz` on the [Releases](https://github.com/LMOxide/lmoxide-discord-bot/releases) page (glibc 2.36 or newer: Debian 12, Ubuntu 24.04) |

LMOxide's `docker-compose.yml` runs the image next to the server, behind its `discord` profile
([HOSTING.md](https://github.com/LMOxide/lmoxide/blob/main/docs/HOSTING.md)).

## Development

[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): a local API to develop against, tests, and the API
contract. API changes the bot needed are recorded in [docs/API-PROPOSALS.md](docs/API-PROPOSALS.md).
