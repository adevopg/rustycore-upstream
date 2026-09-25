# Configuration

RustyCore prefers Trinity-style lowercase configuration names:

| Service | Preferred file | Optional override directory |
|---|---|---|
| Battle.net | `bnetserver.conf` | `bnetserver.conf.d/` |
| World | `worldserver.conf` | `worldserver.conf.d/` |

Use these lowercase names for normal startup. The world service retains a legacy
`WorldServer.conf` fallback, but the Battle.net service does not automatically fall back to
`BNetServer.conf`; a non-default filename must be selected explicitly with `--config`.

Configuration commonly includes database endpoints, listener addresses, data paths, logging,
and runtime feature switches. Keep real credentials, PEM files, database URLs, and local
configuration out of Git; only sanitized examples belong in the repository.

## bnetserver.conf: in-game browser (`Browser.*`)

Ported from the LegionCore fork (`worldserver.conf.dist` there; RustyCore reads them from
`bnetserver.conf` because the Battle.net server answers
`AuthenticationService.GenerateWebCredentials`). Both tables are created by the auth
migrations (`auth.browser_url_map`, `auth.battlenet_account_web_token`).

| Key | Default | Meaning |
|---|---|---|
| `Browser.Enabled` | `0` | `1` persists the credential returned by `GenerateWebCredentials` (method 8) into `auth.battlenet_account_web_token` as a kind `0` (web credentials) token so the shop/support web can validate it. `0` keeps the previous behavior: the login ticket is returned and nothing is stored. |
| `Browser.TokenLifetime` | `3600` | Seconds a persisted in-game browser token stays valid (`expires = NOW() + lifetime`). Expired rows are purged opportunistically after each issue. |

`GET /bnetserver/browser/urlmap/` (same HTTPS listener as `/bnetserver/login/`) serves
`auth.browser_url_map` as a plain JSON object `{"host":"target",...}` regardless of
`Browser.Enabled`; an empty or missing table yields `{}`.
