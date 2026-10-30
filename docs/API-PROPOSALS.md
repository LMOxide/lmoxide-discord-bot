# Proposed API changes

> **All seven items are built on lmoxide's `discord-api` branch (`f641153`), not yet merged into
> main.** The web UI side is on lmoxide-webui's `discord` branch (`d43fdf6`). The bot's live test
> (`docs/DEVELOPMENT.md`) passes against that branch. Beyond the proposal, `ApiKey` has an
> `account_id` (account keys show scope `read`, which isn't used), and a bad id, username or
> password in `POST /v1/discord/accounts` answers 422 `invalid`.

Changes to `lmoxide-api` (in the [lmoxide](https://github.com/LMOxide/lmoxide) repo) that the
Discord bot needs. Written 2026-10-29, before the first version of the bot. The API knew nothing
about Discord and had no per-user credentials, so these features had no API routes:

- registering a game account from Discord (and a registration mode that allows *only* that),
- linking an existing game account to a Discord account,
- a per-account API key that the bot uses when it acts for that person,
- a one-time sign-in code, sent by the bot in a DM, for resetting a forgotten password,
- keeping GM levels in step with Discord roles.

Each item says what the bot and the web UI do with it.

## 1. A `discord` key scope

Scopes are ordered `read < moderate < admin`. Add `discord` between `moderate` and `admin`:
everything `moderate` can do, plus the `/v1/discord/*` routes below. The bot's own key gets this
scope, so it doesn't need `admin` (which could also delete accounts, set passwords, make keys and
read IPs). Admin credentials can use the `/v1/discord/*` routes too.

```sh
lmoxide-api key create discord-bot --scope discord
```

Bot: its one key (`LMOXIDE_API_KEY`) has this scope.

## 2. Account API keys

An API key can belong to an account (`api_keys.account_id`). Such a key acts **as that account**:
its scope follows the account's GM level as a login session's does (none, `moderate`, `admin`),
`/v1/me` works with it, and it stops working while the account is banned, or when the account is
deleted or unlinked from Discord. The audit log names the account and the key:
`account:<username> via api-key:<name>`. `GET /v1/api-keys` lists them with their `account_id`.

The bot makes one for each linked account (`POST /v1/discord/links/{discord_id}/api-key`, item
4) and uses it for that person's commands, such as `/kick` or `/announce`. The API then decides
what that person may do. The bot doesn't have to repeat its permission checks, and the audit log
records the actual account rather than the bot's own key.

## 3. Discord links on accounts

`accounts.discord_id` (unique; a snowflake, sent as a **string** in JSON because it doesn't fit
in a JavaScript number). `Account` gains `discord_id: string | null`.

## 4. Routes for the bot (scope `discord`, tag `discord`)

| Method | Path | |
|---|---|---|
| GET | `/v1/discord/links` | every linked account: `[{discord_id, account_id, username, gm_level, banned}]` (`DiscordLink`) |
| GET | `/v1/discord/links/{discord_id}` | one `DiscordLink`, or 404 |
| POST | `/v1/discord/accounts` | `{discord_id, username, password, gm_level?}` → 201 `{link: DiscordLink, key: CreatedApiKey}`: a new account linked to the Discord account, with its account key. 403 `registration_closed` when registration is `closed` (item 6); 409 `conflict` when the username is taken or the Discord account is already linked |
| PATCH | `/v1/discord/links/{discord_id}` | `{gm_level}` → `DiscordLink`; audit `account-gm` |
| DELETE | `/v1/discord/links/{discord_id}` | unlinks and revokes the account's keys; the account stays |
| POST | `/v1/discord/links/{discord_id}/api-key` | → 201 `CreatedApiKey`: a new account key named `discord-bot`, which replaces (revokes) the account's earlier one of that name |
| POST | `/v1/discord/codes` | `{discord_id, purpose: "link" \| "login"}` → 201 `{code, expires_at}` (item 5) |

The bot uses PATCH only on linked accounts, so it can't change the GM level of an account that
isn't tied to Discord.

## 5. One-time codes, redeemed in the web UI

`POST /v1/discord/codes` makes a short code (`XXXX-XXXX`, letters and digits that can't be
confused with each other) that lasts 10 minutes and works once. A new code for the same Discord
account and purpose replaces the old one. The bot only ever sends it to that person in a DM. Only
the code's SHA-256 is stored.

- **`link`** (the Discord account must not be linked yet, else 409): redeemed by a signed-in
  user with `POST /v1/me/discord {code}` → `Account`, which links their account to that Discord
  account. 409 if their account is already linked.
- **`login`** (the Discord account must be linked, else 404): `POST /v1/auth/code
  {code, remember?}` → `Session` for the linked account. The session has `password_reset: true`,
  and with it `POST /v1/me/password` accepts a missing `current_password` (so `ChangePassword
  .current_password` becomes optional). It doesn't work for a banned account.

Wrong codes count as failed logins (the same per-address lockout as `/v1/auth/login`), and the
error is 401 `bad_code`.

Bot: `/link` and `/reset-password` DM a code with a link to the web UI. After a `/link` the bot
checks `GET /v1/discord/links/{id}` every few seconds until the code expires (the API has no
events), then makes the account key and sets the GM level and roles.

Web UI: an "enter your Discord code" box on the account page, and a "Sign in with a code from
Discord" form on the login page that goes straight to "choose a new password".

## 6. Registration modes

`--registration open|discord|closed` (`LMOXIDE_API_REGISTRATION`, default `closed`).
`--open-registration` stays as a shorthand for `open`.

- `open`: anyone can use `POST /v1/auth/register`, and the bot can register people.
- `discord`: `/v1/auth/register` answers 403 `registration_closed`. Only the bot creates accounts,
  through `POST /v1/discord/accounts`, so everyone who registers is a member of the Discord
  server (the bot checks this).
- `closed`: neither. Only admins create accounts.

`GET /v1/config` gains `registration: "open" | "discord" | "closed"`. `registration_open` stays
(true only for `open`).

Web UI: in `discord` mode, the register page says to use `/register` in the Discord server.

## 7. Dev backend

`tools/web-dev-backend.sh` also creates a `demo-discord-bot` key (scope `discord`).
