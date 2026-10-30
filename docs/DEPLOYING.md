# Deploying the bot

## 1. The Discord application

1. <https://discord.com/developers/applications> → **New Application**. Under **Bot**, reset
   and copy the token (`DISCORD_TOKEN`).
2. Under **Bot → Privileged Gateway Intents**, turn on **Server Members Intent**. The bot needs
   it to see role changes, joins and leaves, and to list members for the sync.
3. Invite it with **OAuth2 → URL Generator**: scopes `bot` and `applications.commands`, bot
   permission **Manage Roles**. Or use this link with the application id filled in:
   `https://discord.com/oauth2/authorize?client_id=<APPLICATION_ID>&scope=bot+applications.commands&permissions=268435456`
4. In **Server Settings → Roles**, drag the bot's role **above Alpha Tester**. Discord only lets
   a bot give roles below its own.

The commands are registered in the configured server when the bot starts (they show up at once).

## 2. The API key and registration mode

On the LMOxide server:

```sh
lmoxide-api key create discord-bot --scope discord
# in Docker:
docker compose exec lmoxide lmoxide api key create discord-bot --scope discord
```

The secret is printed once; that's `LMOXIDE_API_KEY`. To make Discord the only way to register,
run the API with `--registration discord` (`LMOXIDE_API_REGISTRATION=discord`, or
`registration = "discord"` in the `[api]` section of `lmoxide.toml`).

## 3. Settings

`config.toml` (from `config.example.toml`) holds the server id, role ids, bot admins, GM levels
per role, the API and web UI addresses and the sync interval. Secrets are not in it:
`DISCORD_TOKEN` and `LMOXIDE_API_KEY` come from the environment, from files named by
`DISCORD_TOKEN_FILE` / `LMOXIDE_API_KEY_FILE`, or from `discord_token_file` / `api_key_file` in
the settings. `LMOXIDE_API_URL` overrides `api_url`.

`/bot reload` re-reads the settings (roles, GM levels, admins, URLs, sync interval) without a
restart, then `/bot sync` applies role changes. A new `guild_id` or `api_url` needs a restart.

The bot's database (`data_dir`, default `data/` next to the settings) holds the members' account
keys. Each one acts as that member's game account, so keep the directory private and back it up.
If it's lost, the bot makes new keys at the next sync.

## 4. Docker, next to the LMOxide stack

LMOxide's `docker-compose.yml` already has the bot, behind the `discord` profile: see
[HOSTING.md](https://github.com/LMOxide/lmoxide/blob/main/docs/HOSTING.md) in lmoxide. It runs
the published image `ghcr.io/lmoxide/lmoxide-discord-bot`. To build the image yourself:

```sh
docker build -t lmoxide-discord-bot .
```

The service looks like this:

```yaml
  discord-bot:
    image: ghcr.io/lmoxide/lmoxide-discord-bot:latest
    restart: unless-stopped
    environment:
      LMOXIDE_API_URL: http://lmoxide:8080
      DISCORD_TOKEN_FILE: /run/secrets/discord_token
      LMOXIDE_API_KEY_FILE: /run/secrets/discord_bot_key
    volumes:
      - ./discord-bot/config.toml:/etc/lmoxide-discord-bot/config.toml:ro
      - discord-bot-data:/data
    secrets: [discord_token, discord_bot_key]
    depends_on: [lmoxide]
```

with `data_dir = "/data"` in that `config.toml`, a `discord-bot-data:` volume and the two
secrets declared. The `lmoxide` process serves the API at the root of its HTTP listener (8080,
shared with the patch server), so the bot reaches it on the compose network without going
through Caddy.

The bot only connects out (to Discord and the API); it opens no ports.
