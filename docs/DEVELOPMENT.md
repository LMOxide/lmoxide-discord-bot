# Developing the bot

Rust (edition 2024), [poise](https://github.com/serenity-rs/poise) on serenity for Discord,
reqwest for the API, rusqlite for the bot's own database.

| File | |
|---|---|
| `src/main.rs` | settings, secrets, the shared `Data`, starting the client |
| `src/config.rs` | the settings file (`config.example.toml` is tested to parse) |
| `src/api.rs` | the LMOxide API client: the routes the bot uses, error messages for people |
| `src/store.rs` | `data/bot.db`: the account key held for each linked member |
| `src/sync.rs` | roles → GM levels, Alpha Tester role, account keys; member events; periodic sync |
| `src/commands/` | slash commands: `account.rs` (register, link, reset-password, account), `info.rs`, `moderation.rs`, `admin.rs` (`/bot`) |
| `src/live_tests.rs` | the API client against a running API |

## A backend to develop against

From the [lmoxide](https://github.com/LMOxide/lmoxide) repository:

```sh
tools/web-dev-backend.sh          # --reset for a fresh database
```

It serves the API on `http://127.0.0.1:8090`, with demo accounts `admin`, `moderator`, `player`
and `banned` (password `<name>-password`) and a `demo-discord-bot` key (scope `discord`) in
`../var/web-dev/demo-discord-bot-key.txt`.

```sh
cp config.example.toml config.toml   # point guild_id and the roles at a test server
export DISCORD_TOKEN=...             # a test application's bot token
export LMOXIDE_API_KEY=$(grep -o 'lmox_[0-9a-f]*' ../var/web-dev/demo-discord-bot-key.txt)
RUST_LOG=info,lmoxide_discord_bot=debug cargo run
```

## Tests

```sh
cargo test                            # unit tests; the live test skips itself
LMOXIDE_TEST_API=http://127.0.0.1:8090 \
LMOXIDE_TEST_KEY=$(grep -o 'lmox_[0-9a-f]*' ../var/web-dev/demo-discord-bot-key.txt) \
  cargo test live                     # registration, account keys, GM levels, codes, links
cargo clippy --all-targets
```

The live test registers a throwaway account (`bot_test_...`), links and unlinks the demo
`player`, and deletes its account at the end.

## The API contract

lmoxide's `docs/openapi.json` describes every route. The bot's types in `src/api.rs` are written
by hand for the routes it uses. If the bot needs something the API lacks, add it to the API
in lmoxide (and regenerate `openapi.json`) rather than working around it. Record the change in
[API-PROPOSALS.md](API-PROPOSALS.md).

The API has no push events, so the bot polls: `GET /v1/discord/links/{id}` every 5 s while a
`/link` code is open, and a full sync every `sync_interval_minutes`. The API allows 10 requests/s
per address (bursts of 60). The sync pauses between changes.
