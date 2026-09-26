# bnet-shop — checkout web de la tienda in-game (SumUp, sin webhooks)

Servidor HTTPS en Node que sustituye a `www.battle.net/shop/simplecheckout` y
`nydus.battle.net/Bnet/client/purchase/jsutil` para el cliente 7.3.5 (26972).
El worldserver crea el pedido (`Bpay.WebCheckout = 1`), el cliente abre el
navegador CEF con `scene.purchaseRequest` (token SSO + `externalTransactionId`
+ `serverValidationSignature`), esta web cobra con SumUp y marca el pedido como
pagado (`auth.battlepay_purchase.status = 1`); el worldserver lo entrega al
recibir `CMSG_BATTLE_PAY_PURCHASE_SUBMITTED` o al abrir la tienda.

## Flujo

1. `GET /shop/simplecheckout/loading` — página de arranque; espera
   `scene.purchaseRequest` y redirige a `/shop/checkout`.
2. `GET /nydus/Bnet/client/purchase/jsutil` — define `SimpleCheckoutUtil`
   (`getCheckoutUrl/getNavbarUrl/isShopUrl`) por si el proxy usa su propia
   página embebida.
3. `GET /shop/checkout?token=&ext=&sig=&locale=` — valida el token
   (`auth.battlenet_account_web_token`, misma cuenta que el pedido, no
   caducado), crea el checkout SumUp (`POST /v0.1/checkouts`,
   `checkout_reference = external_id`) y monta el widget de tarjeta.
4. Tras el pago el navegador hace polling de `POST /shop/api/status`, que
   consulta `GET /v0.1/checkouts/{id}` hasta `status == "PAID"` → `UPDATE …
   SET status = 1, paid = NOW(), web_order_id = <transaction_code>` (solo si
   sigue en 0, idempotente) → `scene.notifyPurchaseSubmitted({"OrderStatus":1,
   "GlobalOrderId":"<transaction_code>"})` y cierre de la ventana.
5. `POST /shop/api/cancel` re-comprueba SumUp y, si no está pagado, marca
   status 3 y llama a `scene.requestCancelPurchase()`.

No hay webhooks: todo se resuelve por polling desde el navegador y, si el
jugador cierra la ventana a medias, al reabrir el checkout se vuelve a
consultar SumUp (`payment_ref = sumup:<id>`) antes de crear otro.

## Instalación (tera64)

```sh
mkdir -p ~/legion_shop/certs && cd ~/legion_shop
rsync -a tools/bnet-shop/ ~/legion_shop/
npm install --omit=dev
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -keyout certs/shop.key \
  -out certs/shop.crt -subj "/CN=54.38.249.219" \
  -addext "subjectAltName=IP:54.38.249.219,DNS:localhost"
chmod 600 certs/shop.key
cp shop.env.dist shop.env && chmod 600 shop.env   # rellenar SUMUP_API_KEY / SUMUP_MERCHANT_CODE
./run.sh                                          # o la unidad legion-shop.service
curl -sk https://127.0.0.1:8095/health
```

Las credenciales de MariaDB se leen de `~/legioncore/db.env`
(`LEGION_DB_USER/PASS`). La clave de SumUp (`sup_sk_…`) se crea en
Perfil → Desarrolladores → Claves API; el código de comercio (`M…`) está en la
misma pantalla. Nunca pasarlas por línea de comandos: editar `shop.env`.

El cliente llega aquí a través de `auth.browser_url_map`
(`*.battle.net` → `https://<ip>:8095`, `nydus.battle.net` →
`https://<ip>:8095/nydus`); el shim CEF acepta el certificado autofirmado
solo para los destinos de ese mapa (`urlmap_is_target`).

## Prueba sin cliente

```sql
INSERT INTO auth.battlenet_account_web_token (token,battlenet_account,account,realm,character_guid,program,kind,ip,created,expires)
  VALUES (REPEAT('ab',32),1,1,1,1,5730135,1,'127.0.0.1',NOW(),NOW()+INTERVAL 1 HOUR);
INSERT INTO auth.battlepay_purchase (external_id,signature,battlenet_account,account,realm,character_guid,product_id,price,currency,ip,status)
  VALUES ('1234567890abcdef',REPEAT('cd',16),1,1,1,1,109,100.00,'EUR','127.0.0.1',0);
```

Abrir `https://<ip>:8095/shop/checkout?token=abab…&ext=1234567890abcdef&sig=cdcd…&locale=esES`
en un navegador normal: las llamadas a `scene.*` se ignoran fuera del cliente.

## Soporte / tickets web (`support.mjs`)

El cliente 7.3.5 abre el navegador in-game para "Base de conocimiento",
"Abrir ticket" y "Estado del ticket" (`VISITABLE_URL5/6/7` =
`https://<region>.battle.net/support/<lang>/{games/wow,ticket/submit,ticket/status}`,
p. ej. esES `https://eu.battle.net/support/es/ticket/submit?loc`) **tal cual,
sin ningún SSO previo**; el mapa `auth.browser_url_map` trae al cliente aquí.

### Cómo dispara el SSO el cliente (ingeniería inversa de `Wow-64.exe` 26972 + `WowBrowserProxy.exe`)

El cliente no reconoce ninguna URL de login: el disparador es una **cabecera
HTTP** que `WowBrowserProxy` lee en `CefRequestHandler::OnResourceResponse`
solo cuando la respuesta es **200**:

1. Página protegida sin cookie → `302` a
   `/login/?ref=<URL completa>&app=support` (igual que hacía battle.net).
2. `GET /login/` → `200` con `X-BNET-Authenticate: BattlenetToken
   server-salt="<hex>"` → el proxy manda `SSOSalt(url)` al cliente →
   `BrowserController::RequestSSO` → `CMSG_GENERATE_SSO_TOKEN` (kind 0) al
   worldserver.
3. Con `SMSG_GENERATE_SSO_TOKEN_RESPONSE` el cliente carga
   `<scheme>://<host>/login/sso?token=<tok>&<query de la URL del paso 2>` con
   cabecera `X-GAME-CLIENT: WoW/7.3.5.26972`. **La ruta de la URL original se
   descarta y solo se conserva la query**: por eso el destino de vuelta tiene
   que viajar en `ref=` (y por eso `?token=` en la propia URL de tickets no
   funcionaría, el salt se ignora en 7.3.5).
4. `/login/sso` valida el token, deja la cookie `wowsso` y responde `302` a
   `ref`. Blizzard respondía `200` + `Authentication-Info: BattlenetToken
   server-proof="…"` + `Location` (mensaje `SSOProof`, el cliente carga
   `Location` si pasa la whitelist); un `302` lo sigue CEF sin pasar por el
   cliente y no deja estado pendiente, así que es la vía segura. Un
   `X-BNET-Authenticate: BattlenetToken error-code="…"` en un 200 generaría
   `SSOError`; aquí un token inválido devuelve 403 sin cabecera.

Las páginas de soporte nunca deben incluir esa cabecera en subrecursos ni
cuando ya hay sesión (`/login/` con cookie válida redirige directamente),
para no entrar en bucle de SSO.

La web **nunca escribe en `gm_tickets`**: encola la acción en
`characters.gm_ticket_web_queue` (`action` 0 crear / 1 cerrar / 2 mensaje del
jugador) y el worldserver la aplica en `TicketMgr::Update` cada
`Ticket.WebQueueInterval` segundos (`processed` 1 hecho / 2 rechazado,
`result` = ticketId o código de error). Así los ids y el estado siguen siendo
del core; máximo 3 tickets abiertos por personaje. Cuando el core aplica una
acción (o un GM cierra/responde con `.ticket …`) manda
`SMSG_GM_TICKET_CASE_STATUS` al jugador con la URL `Ticket.WebUrl/<id>`
(`worldserver.conf`), que es la que abre el botón "Ver ticket" del cliente y
la que resuelve `/support/ticket/<id>` (sin idioma; se toma de
`Accept-Language`). La respuesta del GM (`.ticket response append`) se guarda
ahora en `gm_tickets.response` y se muestra en la web.

Rutas:

- `GET /login/[<lang>/]?ref=…&app=support` — si ya hay cookie válida, 302 a
  `ref`; si no, `200` + `X-BNET-Authenticate: BattlenetToken server-salt=…`
  (paso 2) con una página de espera.
- `GET /login/sso?token=…&ref=…` — valida el token, deja cookie `wowsso`
  (Secure/HttpOnly) y redirige a `ref|redirect|next|returnUrl|url|…` (ruta
  local o URL absoluta `https://<región>.battle.net/…`, de la que se toma
  ruta+query) si apunta a `/support` o `/shop`; si no, a
  `/support/<lang>/ticket/status`. Se loguea la URL completa en `shop.out`
  (`sso GET …`).
- `POST /support/api/clientdata` — el JS de la página manda
  `wowClient.getClientData()` si existe; solo se loguea.
- `/support[/<lang>][/games/...]` — base de conocimiento (sin sesión).
- `/support/<lang>/ticket/submit` — formulario (título ≤100, texto ≤4000). Si
  el token no lleva personaje, selector `?char=<guid>` entre los de la cuenta.
- `/support/<lang>/ticket/pending?id=<cola>` — espera a que el core procese la
  fila y salta al ticket.
- `/support/<lang>/ticket/status` — lista de tickets del personaje.
- `/support/<lang>/ticket/<id>` — detalle; `POST` con `body=` añade un mensaje
  (acción 2) y `action=close` lo cierra (acción 1).

Requiere `sql/updates/characters/2026_09_04_00_gm_tickets_web.sql` y
`SHOP_DB_CHARACTERS` en `shop.env`.

Prueba sin cliente (con un personaje real de la cuenta 1, guid `G`):

```sql
INSERT INTO auth.battlenet_account_web_token (token,battlenet_account,account,realm,character_guid,program,kind,ip,created,expires)
  VALUES (REPEAT('ab',32),1,1,1,G,5730135,0,'127.0.0.1',NOW(),NOW()+INTERVAL 1 HOUR);
```

```sh
C=$(mktemp)
curl -sk -c $C -o /dev/null "https://127.0.0.1:8095/login/sso?token=$(printf 'ab%.0s' $(seq 32))&ref=https://us.battle.net/support/es/ticket/submit?loc"
curl -sk -b $C --data-urlencode "title=Prueba" --data-urlencode "body=Texto" -o /dev/null -w "%{redirect_url}\n" "https://127.0.0.1:8095/support/es/ticket/submit?char=G"
# a los ~5 s: SELECT * FROM characters.gm_ticket_web_queue;  SELECT * FROM characters.gm_tickets;
curl -sk -b $C https://127.0.0.1:8095/support/es/ticket/status
```

## Reclutar a un amigo (`raf.mjs`)

Botón "Reclutar a un amigo" de la lista de amigos del cliente 7.3.5 (`FriendsFrame.xml`
`RaFButton`, visible cuando `SMSG_FEATURE_SYSTEM_STATUS.RecruitAFriendSendingEnabled` = 1, es
decir `RecruitAFriend.Enabled = 1` en `worldserver.conf`). El diálogo (`RecruitAFriendFrame.lua`)
manda `CMSG_RECRUIT_A_FRIEND` = `bits7 nombre, bits9 email, bits10 nota + strings` y el core
responde `SMSG_RECRUIT_A_FRIEND_RESPONSE` (`bits3 Result`: 0 ok, 1 límite de cuenta, otro fallo;
el cliente muestra `ERR_RECRUIT_A_FRIEND_ACCOUNT_LIMIT`/`ERR_RECRUIT_A_FRIEND_FAILED`).

1. El worldserver guarda la invitación en `auth.battlenet_account_raf_invitations` (token de 32 hex,
   reclutador, reino, facción, email, nota) tras validar el email y el límite
   `RecruitAFriend.MaxInvitations` de invitaciones pendientes por cuenta Battle.net.
2. `raf.mjs` envía cada 30 s las invitaciones con `status = 0` por SMTP (`RAF_SMTP_*`; sin
   `RAF_SMTP_HOST` deja el enlace en el log) con el enlace `<RAF_PUBLIC_URL>/raf/<token>`.
3. La página del enlace muestra quién invita y crea la cuenta Battle.net (email = usuario,
   `salt`/`verifier` SRP6 v1 igual que `Battlenet::AccountMgr::CreateBattlenetAccount`, BattleTag
   elegido + `#dddd`) y la cuenta de juego `<id>#1` con `account.recruiter` = cuenta de juego del
   reclutador (activa las bonificaciones RAF existentes del core: experiencia, invocar, conceder
   niveles) y marca la invitación `status = 2`.
4. `Battlenet::FriendsMgr::Update` (cada `RecruitAFriend.QueueInterval` s) ve las aceptadas, hace
   amigos de BattleTag a los dos (`battlenet_account_friends`, notificación y presencia en vivo) y
   avisa al reclutador por chat ("Your recruited friend is now your Battletag Friend: [Tag#1234]").
   El evento `RECRUIT_A_FRIEND_INVITER_FRIEND_ADDED` del Lua ya no tiene ninguna ruta de disparo en
   el cliente 26972, por eso se emula con un mensaje de sistema.

## Autenticador (`authenticator.mjs`)

Página que explica cómo vincular un autenticador TOTP (Google Authenticator, Authy, la app de
Battle.net...) a la cuenta y así conseguir 4 huecos de mochila extra. Rutas:
`GET /account/authenticator` y `GET /account/authenticator/<lang>` (`es`/`en`; también por
`?locale=` o `Accept-Language`).

La abre el botón **"Activar"** del popup de la mochila (`BACKPACK_INCREASE_SIZE` →
`LoadURLIndex(41)` → `VISITABLE_URL41` de `GlobalStrings.db2`, hotfixeada a esta URL en
`sql/updates/hotfix/2026_09_10_00_global_strings_authenticator_url.sql`). Ese botón abre el
**navegador externo** del sistema (no el CEF de la tienda), así que el `urlmap` del shim no la
intercepta: hay que apuntar `VISITABLE_URL41` a esta página directamente. El hotfix incluye la fila
`global_strings_locale` para `esES` porque el core carga primero la cadena localizada del `.db2` del
cliente; sin ella, un cliente no-enUS seguiría abriendo la URL `nydus.battle.net` original (ver el
cambio de orden en `DB2Stores.cpp::LoadDB2`, que hace que un hotfix de locale gane sobre el fichero).

La vinculación se hace **desde el juego** con `.bnetaccount authenticator on` (genera y muestra la
clave), se escanea el QR (generado en el navegador, la clave nunca se envía al servidor) y se
confirma con `.bnetaccount authenticator confirm <código>`. Para quitarlo:
`.bnetaccount authenticator off <código>` (los ítems de los 4 huecos extra se envían por correo).

Variable opcional: `AUTHENTICATOR_ISSUER` (nombre que se ve en la app; por defecto `shopName`).
En el bnetserver: `Authenticator.RememberDeviceDays`, `Authenticator.MaxAttempts`,
`Authenticator.LockoutSeconds`; en el worldserver: `Authenticator.Issuer`, `Authenticator.WebUrl`.

## RustyCore (WotLK Classic 3.4.3.54261)

Este directorio es la copia de la tienda que sirve al reino RustyCore. Mismo codigo que la de
LegionCore 7.3.5 con estas diferencias, todas compatibles con el cliente 7.3.5:

- **Bases de datos**: `rc_auth`, `rc_world`, `rc_characters` (`shop.env.dist`). Las tablas
  `battlenet_account_raf_invitations`, `battlenet_account_recovery` y `battlenet_account_sms`
  no existen en RustyCore; los sondeos de `raf.mjs`, recuperacion y SMS solo escriben un aviso
  en el log cada pocos segundos.
- **Navegador integrado 3.4.3** (`bnl_checkout 5.3.4`): el cliente no inyecta `scene.purchaseRequest`.
  La pagina de carga pide el pedido con `GET https://blz-data/purchaseRequest` (como la pagina
  oficial `blizzard-checkout/loading`), y los mensajes de vuelta (`purchaseSubmitted`,
  `orderComplete`, `purchaseError`, `purchaseCanceledBeforeSubmit`, `windowCloseRequested`) se
  envian con `POST https://blz-data/<tipo>` y `Content-Type: text/plain` (con `application/json`
  el preflight CORS los bloquea). El nombre del campo del tipo no se pudo observar, asi que se
  mandan las variantes `type`/`code`/`message`/`event`/`name`/`messageType` (`blzPost`).
- **URLs del cliente**: el ejecutable se parchea con `tools/wow-url-patch` (checkout ->
  `/shop/simplecheckout/loading`, navbar -> `/nb`, lista blanca `nightspire.gg`).
- **Tema claro** (`ui.mjs`, `CSS_CLARO`, `opts.tema = "claro"`) en las paginas que se abren
  dentro del juego: carga, checkout, pagado, error y navbar.
- `start-shop.example.ps1`: arranque en Windows como segunda instancia (puerto 8096) junto a la
  tienda de LegionCore (8095), cargando `shop.env` y sobrescribiendo lo que cambia.
