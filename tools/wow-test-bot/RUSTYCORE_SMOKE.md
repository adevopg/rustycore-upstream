# RustyCore smoke tests

This bot can be used as a headless client harness for RustyCore. Treat it as an
E2E regression tool only: C++ TrinityCore remains the protocol/source-of-truth.

## Bounded login gate — live and mutating

The first RustyCore gate is login-only:

```text
BNet auth -> world auth -> CMSG_ENUM_CHARACTERS -> CMSG_PLAYER_LOGIN -> SMSG_LOGIN_VERIFY_WORLD
```

This intentionally does not run Dungeon Finder/LFG. LFG is a later gate, after
the corresponding Rust server port is ready.

Login is not read-only: authentication/session state and normal player login/logout
can write the databases. The wrapper may also create missing disposable bot identities
and an ignored password file by default. Use only authorized test identities and the
approved runtime/DB targets; disabling bootstrap does not make gameplay read-only.
Runtime swaps and destructive fixture modes retain their separate explicit guards.

## Character provisioning on a fresh realm — live and mutating

Every other workflow needs a `characters` row to exist: the preflight refuses to
run without one, and `config.example.json` points at a guid from a pre-existing
development database. On a freshly bootstrapped realm there is none, and the run
stops with `No characters row for guid <n>`.

`--create-character` provisions one over the wire — no SQL insert:

```bash
set -a; . ./.env.local; set +a   # or export WOW_BOT_PASSWORD_… yourself
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --create-character-name Rustyqa
```

It runs alone (no other workflow flag) and needs exactly one enabled bot. The
sequence is:

```text
BNet auth -> world auth -> CMSG_ENUM_CHARACTERS -> CMSG_CREATE_CHARACTER
  -> SMSG_CREATE_CHAR -> CMSG_ENUM_CHARACTERS -> read back the row
  -> CMSG_PLAYER_LOGIN -> SMSG_CONNECT_TO -> SMSG_LOGIN_VERIFY_WORLD
  -> CMSG_LOGOUT_REQUEST -> SMSG_LOGOUT_COMPLETE -> read back the row
```

The first login is part of the mode on purpose: C++ character creation sets
`AT_LOGIN_FIRST` (`Handlers/CharacterHandler.cpp:888`) and only the first
`HandlePlayerLogin` clears it (`CharacterHandler.cpp:1271`), so a character that
has never logged in is not yet the clean offline fixture the other workflows
require. The mode prints the guid the server assigned; put it in
`character_guid` in your ignored local `config.json`.

Defaults are race 1 (Human) and class 1 (Warrior); override with
`--create-character-race`, `--create-character-class` and
`--create-character-sex`. The name must be 2–12 ASCII letters, and the server
still has the final word: a refusal is reported with its C++ `ResponseCodes`
name, for example `CHAR_CREATE_NAME_IN_USE`.

This mode writes: it creates an account fixture if missing, writes
`account.session_key_bnet` and `account.os`, and creates a character. Use it only
against authorized test identities.

### Looting the kill

`--loot-after-kill` adds the loot phase to `--melee-smoke`: open the corpse, take
the money, take every item the window offers, close it, then log out cleanly and
read back what was granted.

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --loot-after-kill --melee-creature-entry 94 --melee-timeout 120
```

`CMSG_LOOT_ITEM` must quote the **LootObject** guid, which is the *second* guid in
`SMSG_LOOT_RESPONSE`, not the creature's. The handler resolves the request through
`active_loot_owner_for_loot_object_like_cpp`, so the creature's own guid finds
nothing and is answered with a bare `SMSG_LOOT_RELEASE` — no error, no item, and
nothing in the log. `CMSG_LOOT_RELEASE`, by contrast, carries the owner.

The loot list id is read back from the response rather than assumed: the server
assigns it over every generated entry (`handlers/loot/generation.rs:172`) and
resolves the request by it (`handlers/loot/authority.rs:127`), so entries this
player never sees still consume ids.

Item blocks carrying item bonuses or modifications are refused rather than guessed,
because their lengths are variable; white loot never produces them.

Pick the target by what it drops. `creature_template_difficulty` holds `GoldMin`,
`GoldMax` and `LootID`: entry 299 (Diseased Young Wolf) has no gold at all, so
`coins=0` there is correct data and not a defect, while entry 94 (Defias Cutpurse,
1-12 copper, spawns 25 yards from the QA start) exercises the money half.

## Area-trigger quest credit — live and mutating

`--area-trigger` drives C++ `WorldSession::HandleAreaTriggerOpcode`'s quest block
(`Handlers/MiscHandler.cpp:530-574`) and reports only what the server published.

The trigger's geometry is **client data**: `AreaTrigger.db2`, not SQL, and
`hotfixes.area_trigger` is empty on this installation. Guessing the coordinates
would be inventing data, so ask the server for the ones it loaded:

```bash
RUSTYCORE_AREA_TRIGGER_TRACE=87,88 ./target/debug/world-server
# RUST_AREA_TRIGGER geometry id=87 continent_id=0 x=-9077.34 y=-552.92 z=60.35 radius=30
```

Then stand in it and send the packet:

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --area-trigger 87 --area-trigger-quest 76 \
  --area-trigger-at -9077.34,-552.92,60.35 --area-trigger-map 0
```

The sequence is `login -> CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE -> CMSG_AREA_TRIGGER
(Entered) -> observe SMSG_QUEST_UPDATE_ADD_CREDIT_SIMPLE and
SMSG_QUEST_UPDATE_COMPLETE on both sockets -> clean logout -> read
character_queststatus and character_queststatus_objectives back`. A run fails if
the server published neither a credit nor a completion.

`CMSG_AREA_TRIGGER`'s two bits are written most-significant first, matching the
server's `WorldPacket::write_bit`: `Entered` is bit 7 and `FromClient` bit 6 of the
single flushed byte.

Exactly one `SMSG_QUEST_UPDATE_COMPLETE` is correct for a quest that carries both
`QUEST_FLAGS_COMPLETION_AREA_TRIGGER` and a `QUEST_OBJECTIVE_AREATRIGGER`:
`Player::CompleteQuest` (`Entities/Player/Player.cpp:14947-14971`) publishes
nothing, and the one packet comes from `AreaExploredOrEventHappens`'s own
`SendQuestComplete` (`:16507`). Two meant the objective path was sending a second
copy, which is how that divergence was found.

Reference run on quest 76 "The Jasperlode Mine", trigger 87:
`simple_credits=1 quest_completes=1 objective_data=Some(1) explored=Some(1)
status=Some(1)`.

It also restores a character a previous run left dead, and says so with
`revived=true`. That is not optional: C++ gates the whole quest block on
`player->IsAlive()` (`Handlers/MiscHandler.cpp:530`), the refusal is invisible on
the wire — a dead character just produces zero credits, which reads exactly like a
broken server — and a trigger position is *where the quest's mobs are*, so parking
a level-2 character at the Jasperlode Mine for ten seconds is usually fatal. One run
failed that way before the fixture existed, and the server was right to refuse it.

This mode writes: it seeds the quest as incomplete, restores the character's health
if it is zero, and moves the character into the trigger — all fixtures to columns
`Player::SaveToDB` owns. The original map and position are restored whether the run
passes or fails; the quest rows are left as the server wrote them, because they are
the evidence.

## Aura persistence across a logout — live and mutating

`--aura-save` drives C++ `Player::_SaveAuras` (`Entities/Player/Player.cpp:20089-20146`),
which the full save reaches right after `_SaveActions` (`:19948`).

The hard part of this check is that the obvious observation proves nothing. Seeding a
`character_aura` row, logging in and out, and finding the row still there looks exactly
the same whether the save rewrote it or never ran at all — and never running was the
actual defect. So the fixture seeds **two** rows and the assertions are about the
difference between them:

* the spell you pass, with stored values the save cannot reproduce: `remainCharges = 5`
  and `baseAmount = 777777`. After the logout both must be gone, because `_SaveAuras`
  rewrites every row from the live aura.
* spell `90000001`, which no `Spell.db2` row can carry. `_LoadAuras` drops a stored aura
  whose `SpellInfo` is missing, so it is not in the live aura map and the save must not
  write it back. If that row survives, the table was never cleared.

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local --aura-save 6673
```

The sequence is `seed -> login -> count SMSG_AURA_UPDATE -> clean logout -> read
character_aura and character_aura_effect back -> login again -> count SMSG_AURA_UPDATE`.
A run fails if the unknown-spell row survived, if the seeded `remainCharges` or
`baseAmount` is still there, if the spell is missing from the table, or if the relog
published no aura update.

Pick a spell the character actually knows nothing about — the aura is restored from the
row, not cast, so any spell with a `Spell.db2` entry works.

This mode writes: it clears and seeds both aura tables for the QA character and restores
one a previous run left dead. The rows after the run are left as the server wrote them,
because they are the evidence.

## Player spell damage, criticals and resists — live and mutating

`--spell-damage` is the mode three closed entries were waiting on. A spell critical
needs a caster-side percentage (`Unit::SpellCritChanceDone`, `Unit.cpp:7706-7772`)
and a spell resist needs a magic school (`Unit::CalcSpellResistedDamage`,
`:1973-1975`), so **neither can be reached with a warrior**, however it is
fixtured. The mode drives a caster-class character and reports every published
field of every cast, drawing no conclusion in the harness.

Provision the caster once:

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --create-character-name Rustymage --create-character-race 1 --create-character-class 8
```

That character needs **no spellbook fixture** for a class spell. Its
`character_spell` rows are zero and it still knows 43 spells, Fireball and
Frostbolt among them: C++ `_SaveSpells` writes only **non-dependent** rows
(`Entities/Player/Player.cpp:20664-20666`) and `LearnSkillRewardedSpells` learns
dependent ones, so that table is the wrong place to look for what a character
knows. Start the server with `RUSTYCORE_KNOWN_SPELLS_TRACE=1` to see the granted
set on `SMSG_SEND_KNOWN_SPELLS`.

Seeding is therefore opt-in through `--spell-damage-seed-spell`, for a spell the
login genuinely does not grant; inserting a row for a dependent one would write
something the target build never writes. A level-1 caster has the mana for only a
couple of casts, so a `characters.level` fixture is worth applying by hand for a
longer sequence.

```bash
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --spell-damage 133 --spell-damage-entry 475 --spell-damage-character 6 --spell-damage-casts 1
```

The sequence is `resolve the spawn from world.creature -> seed the spellbook row and
stand 12 yards off -> login -> CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE -> read the
CREATE block for the runtime ObjectGuid -> CMSG_CAST_SPELL -> record every
SMSG_SPELL_NON_MELEE_DAMAGE_LOG -> clean logout -> restore the position`. A run
fails if the server published no damage log at all.

Reference runs, Fireball (133) at a Kobold Tunneler (entry 475, 21 fire resistance
in `creature_template_resistance`), the second with `character_spell` emptied to
zero rows and no seeding at all:
`cast 1: damage=11 original=13 resisted=2 absorbed=0 school=0x04 flags=0x00` and
`cast 1: damage=12 original=13 resisted=1 absorbed=0 school=0x04 flags=0x00`.
The average reduction is `21 / (21 + 100) = 0.17`, whose discrete table puts the
weight on the one- and two-tenth buckets, and `13 * 2/10` truncates to the 2 the
server published. `flags` stays zero for a non-critical hit, and it stays zero for
a critical resist too: C++ writes that field in **seven bits**
(`CombatLogPackets.cpp:39`) while `HITINFO_*_RESIST` are `0x80` and `0x100`, so the
client learns of a resist from `Resisted` and never from the flags.

Two things to know before reading a zero-log run as a defect:

* **the target must be alive and in range.** A previous run can leave the nearest
  spawn of that entry dead, and `--spell-damage-entry` then resolves to a corpse:
  the casts complete and deal nothing. Pass a different entry or wait for the
  respawn.
* **a drained caster is refused, correctly.** A session starts with the mana saved
  at the previous logout, so a sequence that looks like "the server dropped my
  casts" is usually `SPELL_FAILED_NO_POWER` — which this mode now reports by name.
  The fixture therefore fills `characters.power1` before login, in the same place
  it revives a dead character; the stored value is clamped to the character's real
  maximum when `InitStatsForLevel` runs, so it means "full" rather than an invented
  pool. The world-pass deadline warning this host logs continuously is unrelated:
  the coordinator allows each session one map tick interval to report, which any
  pass touching the database exceeds by a few milliseconds.

A refusal is reported with its `SpellCastResult`, not just its opcode. `SpellCastVisual`
serialises **one** `uint32` on this branch, so the reason sits four bytes earlier than a
two-field reading puts it — getting that wrong reported every refusal as
`SPELL_CAST_OK`, and a test now pins the offset.

`WOW_BOT_SPELL_DAMAGE_TRACE=1` logs every opcode on both sockets, which is how the
`SMSG_SPELL_START` / `SMSG_SPELL_GO` / `SMSG_SPELL_NON_MELEE_DAMAGE_LOG` order above
was established.

This mode writes: it seeds one `character_spell` row, restores a character a
previous run left dead, and moves the character beside the spawn. The position is
restored whether the run passes or fails; the spellbook row is left, because it is
what makes a repeat run cheap.

## The death exit — live and mutating

`--death-smoke` drives the whole corpse circuit and reports what the server
recorded:

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --death-smoke --death-timeout 120
```

```text
fixture: health = 0 and PLAYER_FLAGS_GHOST cleared, stale corpse rows dropped
login -> CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE -> CMSG_REPOP_REQUEST
      -> SMSG_MOVE_TELEPORT to the graveyard, acknowledged
      -> run back to the position the `corpse` row itself carries
      -> CMSG_RECLAIM_CORPSE every 5 s until it takes
      -> clean logout, then read health and playerFlags back
```

The acceptance signal is the `corpse` row, not a decoded update block. C++
`Corpse::SaveToDB` writes it inside `CreateCorpse` and
`Map::ConvertCorpseToBones` deletes it inside the reclaim
(`Maps/Map.cpp:3748-3750`), both committed immediately rather than at the next
player save, so the row appearing and then disappearing is the server's own
record of the two transitions. The logout then confirms the player came back:
half health and no ghost flag.

The fixture writes `characters.health = 0` and clears `PLAYER_FLAGS_GHOST`,
because a fresh death is a corpse and not a ghost — C++
`WorldSession::HandleRepopRequest` (`Handlers/MiscHandler.cpp:62-63`) refuses to
release a spirit that already carries that flag, and the flag is persisted. Like
the `--melee-smoke` revive, it is a fixture reset and exercises no server death
path.

Every refusal is silent on the wire by design: C++ returns without a packet. Set
`RUSTYCORE_CORPSE_RECLAIM_TRACE=1` on the server to see which gate refused and,
for the delay, how many seconds are left.

## Retiring QA characters — live and destructive

`--delete-characters <guid,guid,…>` deletes characters through the server's own
`CMSG_CHAR_DELETE` path rather than by SQL, so the deletion follows whatever
`Player::DeleteFromDB` fan-out the server implements:

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local --delete-characters 1,2,3
```

It runs alone, needs one enabled bot, refuses a non-local account without
`WOW_BOT_ALLOW_NONLOCAL_ACCOUNT_BOOTSTRAP=1`, refuses the bot's own configured
`character_guid`, and fails if the server refuses any of the requested deletes. It
enumerates first because the server only accepts a delete for a character it has
listed for that account.

**Known server defect this mode exposes:** the delete currently removes only the
`characters` row and leaves every dependent row behind — inventory, items, skills,
glyphs, reputation, homebind. See the CRIT entry dated 2026-10-01 in
docs/migration/EXISTING-CODE-DEFECTS.md before using it on anything you care about.

## Live melee engagement check — live and mutating

`--melee-smoke` drives the MVP combat loop against a real spawn and reports what
the server published, with nothing inferred:

```bash
set -a; . ./.env.local; set +a
cargo run -- --config config.json --single TESTBOT1@bot.local \
  --melee-creature-entry 721 --melee-creature-guid 279982
```

It runs alone, needs exactly one enabled bot and an existing character, and the
sequence is:

```text
login -> CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE -> walk to the spawn with
CMSG_MOVE_HEARTBEAT while reading SMSG_UPDATE_OBJECT -> discover the live
ObjectGuid -> CMSG_ATTACK_SWING -> observe SMSG_ATTACK_START,
SMSG_ATTACKER_STATE_UPDATE both ways, SMSG_ATTACK_STOP (NowDead) and
SMSG_LOG_XP_GAIN
```

The engagement is observed on **both** sockets. `SMSG_LOG_XP_GAIN` is
`CONNECTION_TYPE_REALM` in C++ (`Server/Protocol/Opcodes.cpp:1662`), so it never
arrives on the instance socket the attack goes out on; reading only that socket
reported `xp=0` for kills the server had already granted and written to
`characters.xp`.

It also restores a QA character that a creature killed, before logging in, and
says so. A dead attacker is refused by C++ `Unit::Attack`
(`Entities/Unit/Unit.cpp:6175`) and RustyCore answers `CMSG_ATTACK_SWING` with
`InvalidDeadAttacker`, which on the wire is indistinguishable from "the server
never published SMSG_ATTACK_START". The server's only implemented exit from death
is `CMSG_REPOP_REQUEST`, and C++ `Player::RepopAtGraveyard` leaves the player a
ghost, so there is no in-protocol way back to a live character yet. The restore
writes `characters.health`, which is where the death state lives
(`Player::LoadFromDB` reads a zero-health row as a corpse, `Player.cpp:18119`, and
clamps the value at the computed maximum, `:18135`). It is a fixture reset, not a
resurrection, and it exercises no server death-exit behaviour.

`CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE` is not optional: C++
`Player::CanNeverSee` (`Entities/Player/Player.cpp:23214-23218`) hides every
object from a player that has not set
`PLAYER_LOCAL_FLAG_OVERRIDE_TRANSPORT_SERVER_TIME`, so without that packet the
world looks empty and no target can be discovered.

The walk is made of ordinary heartbeat steps of at most 20 yards that stop one
yard inside melee reach; it does not teleport. It continues during the
engagement: the mode follows the target's `SMSG_ON_MONSTER_MOVE` position
(`MoverGUID` then `Pos`, `Server/Packets/MovementPackets.cpp:589-594`) and keeps
republishing its facing, because C++ `Unit::DoMeleeAttackIfReady` needs both
`IsWithinMeleeRange` and `HasInArc(2*pi/3, victim)` and answers a standing bot
with `SMSG_ATTACKSWING_ERROR` instead of a swing.

**This mode is not yet reliable against a wandering spawn.** The nearest hostile
spawns to the QA start position (`creature` 280052/280053) both carry
`MovementType = 1` with a 10-yard wander, and the follow only knows where the
spline *started*, so most swing attempts still resolve out of reach: a 60-second
run typically lands one or two swings and often none. A run that lands nothing is
therefore not evidence of a server defect on its own — check the server's
`RUSTYCORE_PLAYER_MELEE_TRACE=1` phase counters before concluding anything. Once
one swing lands the creature engages and chases by itself, so the first connected
swing is what the scenario is really waiting for. The target is selected by
`creature_template.entry`, optionally pinned to one `world.creature.guid`, and its
live ObjectGuid must be discovered within 60 yards of that SQL position — the mode
fails closed rather than attacking a guessed GUID.

Pinning a stationary spawn removes the wander from the picture, and it is the
reliable way to run this mode: `creature.guid` 280092, 280093 and 280091 (entry
94) all carry `wander_distance = 0` and `MovementType = 0`. Doing that on
2026-10-01 exposed a real server defect rather than a harness one — three runs
at 4.0 yards saw no `SMSG_ATTACKER_STATE_UPDATE` at all, because the global
player-melee phase refused every swing with `NotInRange`. It is closed as
**D-M16** in `docs/migration/EXISTING-CODE-DEFECTS.md`, and the same run proved
the fix: `player_landed=4 (45 damage) death=true xp=44 loot_coins=9`.

Read the phase trace, not the summary, when a run lands nothing. `delivered`
counts commands the phase handed to the rail; `queued` and `dropped_durable` say
whether the session received one; the arrival line names the gate that dropped
it; and `ready_attack_without_swing` reports a ready attack that produced no
swing, with the range, facing and unit-state facts behind it. The old
`creature_hits` counter was not usable for this: it counted a result with an
empty swing list as a hit, and now counts only a swing that exists.

A run fails if SMSG_ATTACK_START never arrives or if no player swing lands. The
retaliation, the death and the XP are reported but not required: a critter neither
fights back nor grants XP. With `--report <path>` the summary is written as JSON.

The server-side switches that explain a failure are
`RUSTYCORE_CREATURE_VIS_TRACE=1` (why a creature is or is not visible) and
`RUSTYCORE_PLAYER_MELEE_TRACE=1` (why a swing did or did not happen).

Two more traces belong to the creature spell chain, and both are env-var gated at
info like every other trace in this server rather than `target:`-tagged at debug —
a tagged debug line needs its target named in `RUST_LOG`, which nobody reading this
file would guess:

* `RUSTYCORE_CREATURE_SPELL_TRACE=1` prints one throttled line per tick that saw a
  creature — whether or not anything happened — with every gate as a number: `casts_ready`, `spell_hits`,
  `spell_misses`, `damage_executed`, `damage_unresolved`, `effects_unrepresented`,
  `noninstant`, `projectiles`, and the range, line-of-sight, target, cooldown,
  incarnation and casting-requirement rejections. Without it, a creature that never
  casts is indistinguishable from one the gates refused. It reports on every tick
  with a creature on purpose: "saw thirty creatures, did nothing, every gate at
  zero" is the most useful line it can print, and the first version of this trace
  only spoke when a counter moved, so it stayed silent for exactly that case.
* `RUSTYCORE_CALCVALUE_TRACE=1` prints the inputs of every spell effect's value:
  base points, die sides, per-level term, the spell's `SpellLevels` trio, the
  caster's level and the result. It is what separated the level term from the die
  roll when D-H26 was proven live.

This mode writes: it moves and saves the character's position, and it puts the
character in combat. Use it only against authorized test identities.

## Bounded normal save/relogin check (#578)

With explicit approval to swap/restart `world-server` and use existing
`TESTBOT1@bot.local`, run the maintained two-pass wrapper under the runtime guard:

```bash
QA_SMOKE=/home/server/rustycore/tools/wow-test-bot/run_login_save_relog.sh \
  ./tools/qa-runtime.sh --allow-runtime-qa \
  --world-exec /absolute/path/to/verified/world-server \
  --report /tmp/rustycore-save-relog-runtime.json login
```

Build the bot first. The wrapper inherits the guard's exact bot hash, loopback
endpoints and disabled provisioning/fixture modes. `WOW_BOT_LOGIN_SAVE_CHECK=1`
adds a pre-authentication check of the exact existing Battle.net/game-account
ownership and sole offline character, then normal logout after login-stream
drain. It requires the empty `SMSG_LOGOUT_COMPLETE` response, an offline row and
a strictly newer `logout_time`; socket-loss fallback does not pass. It reads,
but never seeds or cleans, the six spell/favorite/skill/equipment/transmog/
reputation tables. Pre-existing rows must survive unchanged; login defaults may
be added. Two fresh authentications must retain identical saved projections and
known/favorite-spell packets. The private bot report has
`login_save_relog_verified=true` only after both passes, while the outer runtime
report separately records restoration. Ordinary auth/login/logout DB writes
remain, by design. `bnet-server` is not restarted.

This is bounded save/relogin evidence, not whole-character parity, a crash or
unknown-COMMIT experiment, concurrency proof, or a fresh capture. The new QA
module is private `src/login_save.rs`; `test_login_save_relog.sh` tests report
acceptance without a server or database. Missing/mutated existing data is a
failure to investigate, not permission to repair the fixture.

Login draining retains `login_instance_object_update_seen` from the initial
login loop, so consuming the required NPC CREATE there does not require a second
`UPDATE_OBJECT` during the drain. Only an INSTANCE packet grants that evidence;
a REALM packet does not. Both sockets must remain open and quiet for a full second
within the drain's 30-second deadline; every received packet restarts that quiet
period. The drain continues to select cancellation-safe socket peeks before reading
complete encrypted frames.

## Transport disconnect/save/relogin (#585)

### Player cast lifecycle (#589)

The [cast lifecycle guide](CAST_LIFECYCLE.md) describes scripted instant/timed,
queued/replaced and cancelled casts, typed packet acceptance and a second-session
observer. The mode preserves ordinary login/transport and performs no fixture
SQL or provisioning, and uses only pre-existing accounts. Its captured facts and
normal logout are distinct from the acquisition/save/relogin persistence gate
below. An `observe_only` observer session always reports `passed: false`: use the
paired correlation procedure in the guide, never a hand-annotated pass.

### Spell acquisition / save / relogin (#587)

`run_spell_acquisition_relog.sh` uses the same normal-save identity and logout
guards, then verifies the action receipt and fresh known-spell packets. It requires
`WOW_BOT_ACQUISITION_PLAN` pointing to an absolute private JSON file. A trainer plan
has `action: "trainer"`, `expected_spell`, the observed live NPC `guid_low` and
`guid_high`, `trainer_id`, `offer_spell`, and the discounted `fee`. Optional
`gossip_option` is the signed wire GossipOptionID (negative IDs are valid), not
the SQL OptionID/order index; it opens the actual menu and selects that option;
otherwise the driver sends TrainerList directly. A cast plan has
`action: "cast"`, `expected_spell`, a known player `spell`, and the client cast token
`cast_low`/`cast_high`. The explicit target is the logged-in player. The wrapper's
second authentication uses `action: "verify"` and sends no acquisition request.
For runtime GUID discovery, set both GUID numbers to zero and provide
`spawn: {"entry": <NPC entry>, "map": <map ID>, "position": [x,y,z]}` from the
isolated SQL spawn. The bot waits for a unique matching CREATE_OBJECT within three
yards and uses that observed GUID; SQL spawn IDs are never substituted for live
GUID counters. Ambiguous or missing candidates fail before any purchase.

Select a previously unknown, unranked spell whose acquisition does not replace
existing skill/spell rows: this driver retains the existing six-family preservation
contract. Ranked/disabled/profession transitions still require their separate
scoped acceptance; this scenario does not pretend to cover them. Trainer mode
first obtains the matching trainer list, buys the offer, requires learning on the
instance connection and checks a repeated purchase is rejected without a second
fee. Cast mode requires the source cast in the login spellbook. Both verify the
learned spell and expected money after confirmed logout, then across fresh login.
The intermediate database money value is observational only because C++ normally
saves Player money during SaveToDB.

Persistence defaults to `{"kind":"direct_spell"}`: an active, non-disabled
`character_spell` row for the target must survive both logouts. For a reviewed
skill-rewarded target with stable values across login, set the plan's
`persistence` explicitly as `{"kind":"skill","id":<skill>,"value":<value>,"max":<max>}`.
This contract requires the selected skill to be absent before acquisition,
its exact ID/value/max after each logout, no active target spell row, and the target
in the fresh login spellbook. The wrapper carries the same contract into its
verify plan and compares it across reports. `saved_spell` remains the literal
direct-row observation (false here); `observed_skill_root` and
`persistence_verified` record the alternative evidence. All six existing
preservation checks remain required.

The driver does not discover or seed fixture catalogs, relocate characters, grant
spells, provision accounts or capture both servers by itself. Configure a verified
isolated target and action-specific capture before use. Its `.acquisition.json`
report supplements the ordinary save/relogin report; neither report proves fresh
paired C++ packet parity. No fixture IDs from unit tests are live defaults.

For controlled EffectLearnSpell conformance when stock sources auto-learn their
target during login, `prepare_spell_acquisition_data.py --source-data <Data>
--output-data <new-private-Data> --target-spell 6197` creates a separate version-2
data tree. SpellMisc record 336029 / spell 30798 loses Attributes[1]
CAST_WHEN_LEARNED (SQL column `Attributes2`), changing one reviewed byte per
locale. SpellEffect record 705389 retains parent 30798, difficulty 0, effect index 0
and LEARN_SPELL 36; only its `EffectTriggerSpell` changes from 674 to 6197. That
20-bit field begins at record bit 145, and the change affects two bytes per locale.
The generator validates the reviewed enUS/esES/ruRU assets before creating output.
Other assets are read through symlinks; stock files are verified unchanged.
Unknown hashes and an existing output directory are rejected. The manifest records
the source and target spells, version, input/output hashes, record/parent locations
and changed bits. This does not change skill 118 metadata or weaken preservation.

Omitting `--target-spell` (or selecting 674) preserves the original version-1
SpellMisc-only overlay. Keep its previous artifacts as diagnostic evidence:
C++ explicitly learns 674 and the first logout saves skill 118=1/1 without a
direct 674 row. At the next login, stock SkillRaceClassInfo 132 flags 0x92 and
SkillLine 118 category 6 cause `_LoadSkills` → `UpdateSkillsForLevel` to normalize
that skill to 100/100 at level 20. This is expected server behavior; version 1
does not satisfy the driver's unchanged six-family retention contract.

Point both isolated servers at the same overlay and verify effective SQL
SpellMisc, SpellEffect, hotfix and dependency inputs before either run. Seed source
30798 only in the authorized disposable, offline fixture. The existing cast driver
must observe source 30798 known/active and target 6197 absent after login for version 2.
Use `expected_spell: 6197` and the default direct-spell persistence contract;
require explicit-cast learning and ordinary save/relogin retention. Only passing
paired action captures and both retention reports establish conformance with
this controlled metadata; the generator alone does **not** establish stock
30798 gameplay. The generator changes no configuration, database or service;
runtime and fixture authority remain with the caller.

Under the same authorized runtime guard, select
`QA_SMOKE=/home/server/rustycore/tools/wow-test-bot/run_login_disconnect_relog.sh`.
This uses only existing TESTBOT1@bot.local, with provisioning disabled and no SQL
fixture writes or cleanup. The first authentication drains login and sends TCP FIN
on both authenticated transports **without CMSG_LOGOUT_REQUEST**. It waits for a
strictly newer offline character save and an offline Login account, then compares
the same six preserved projections. A second fresh authentication verifies those
projections and spell packets and finishes with ordinary confirmed logout.

The modes `WOW_BOT_LOGIN_DISCONNECT_CHECK=1` and `WOW_BOT_LOGIN_SAVE_CHECK=1`
are mutually exclusive; the wrapper sets each phase explicitly. The private JSON
distinguishes `disconnect_confirmed` from `logout_confirmed`; the aggregate flag is
`login_disconnect_relog_verified`, not ordinary `login_save_relog_verified`.
DB polling allows 90 seconds for legacy socket expiry; the runtime wrapper retains
its overall timeout/restoration guard. This covers orderly transport EOF, not RST,
pending transfer, process crash, uncertain COMMIT recovery or a fresh capture.
`test_login_disconnect_relog.sh` tests report acceptance without a server/database.

### Pending portal transfer (#585)

`run_session_transfer_qa.py --allow-position-fixture --source CLEAN_CHECKOUT
--world-exec CANDIDATE` is the separately authorized positional fixture wrapper.
It pins existing TESTBOT1 (character 14/account 8), requires no online characters,
journals the seven original location fields in a private directory, stops the
world service, positions the character at DB2 trigger 2173, and starts the service.
The existing runtime guard then owns candidate installation and restoration.
After the scenario, the positional wrapper stops the service, restores only those
location fields (not old inventory or account state), and verifies the original
executable is serving. No rows are created/deleted; bnet is not restarted.

The bot sends AreaTrigger enter, observes TransferPending on realm and SuspendToken
on instance, replies to the suspend token, receives NewWorld on realm, then closes
both transports **without WorldPortResponse**. It requires save at the intended map
369 destination, then fresh normal relog/logout at that same saved position.
This is not a completed-teleport logout or an LFG scenario. The first private report
contains the three observed non-secret transfer payloads and the withheld-ACK fact.

Source selection can be inspected read-only through wow-data's
`inspect_finalization_portal DATA_DIR LOCALE 2173` example, using the production DB2
reader. Destination is the existing world relation to safe location 3650. C++ wire
and connection anchors are recorded in `src/login_save/portal.rs`; a Rust-only run
does not establish fresh C++ capture parity.

On interruption, retain the printed private journal. Recovery uses
`python3 run_session_transfer_qa.py --allow-position-fixture --recover JOURNAL`.
The ordinary exception path attempts restoration; SIGKILL/host loss still needs
explicit recovery. Recovery refuses to start a non-original executable or overwrite
an online character. `test_session_transfer_qa.py` exercises the recovery policy
without DB/service access. Do not use this wrapper concurrently with manual play.

## What was adapted

- `--login-only`: verifies world entry and drains the login streams.
- `--quest-smoke`: after login, resolves one creature questgiver, sends
  `CMSG_GOSSIP_HELLO`, falls back to `CMSG_QUEST_GIVER_HELLO`, optionally sends
  `CMSG_QUEST_GIVER_QUERY_QUEST`, and reports the quest ids/titles received.
- `--bank-smoke`: runs a real `CMSG_BANKER_ACTIVATE` → `CMSG_AUTOBANK_ITEM` →
  logout/relogin → `CMSG_AUTOSTORE_BANK_ITEM` → logout persistence round-trip
  using an isolated local fixture item.
- `--void-storage-smoke`: runs unlock/deposit, fresh-login slot swap,
  fresh-login withdrawal, and a final empty-store relog proof against both the
  response packets and CharacterDB.
- `--inventory-swap-smoke`: creates two distinct occupied backpack slots, sends
  `CMSG_SWAP_INV_ITEM`, logs out/re-authenticates, swaps them back, and verifies
  both atomic DB transitions plus exact enchanted/random-property item-create
  metadata before cleaning the isolated local fixture.
- `--rested-xp-smoke`: records a bounded set of fields from one disposable local
  bot character, verifies offline wilderness/resting accrual, attacks a real
  creature, checks `SMSG_LOG_XP_GAIN` and DB consumption, relogs, restores those
  selected fields, and verifies natural target respawn cleanup. It requires the
  CLI-only `--ack-disposable-rested-xp` acknowledgement.
- `WOW_BOT_LOGIN_ONLY=1`: env equivalent of `--login-only`.
- `WOW_BOT_LOGIN_REQUIRE_KNOWN_SPELLS=1`: in login-only mode, keep the
  connection open until `SMSG_SEND_KNOWN_SPELLS` is observed; useful for
  starting-spell capture parity without changing the default login smoke.
- `WOW_BOT_LOGIN_EXPECT_KNOWN_SPELLS=<comma-separated IDs>`: implies the
  previous gate and requires the login packet's canonical known-spell set to
  match exactly. Packet bit padding, counts, length, duplicate/zero IDs and
  favorite membership are validated before the set comparison.
- `WOW_BOT_CLIENT_BUILD` / `WOW_BOT_BUILD`: build value printed by the smoke,
  default `54261`.
- `WOW_BOT_PASSWORD`: shared local password for accounts in `config.example.json`.
- `WOW_BOT_PASSWORD_<ACCOUNT>`: per-account override, with non-alphanumeric
  account characters replaced by `_` and letters uppercased (for example
  `WOW_BOT_PASSWORD_TESTBOT1_BOT_LOCAL`).
- `WOW_BOT_AUTH_DB_URL`, `WOW_BOT_CHAR_DB_URL`, `WOW_BOT_WORLD_DB_URL`: optional
  DB URL overrides. If omitted, the bot reads `LoginDatabaseInfo`,
  `CharacterDatabaseInfo`, and `WorldDatabaseInfo` from `WOW_BOT_DB_CONF`
  (default `/home/server/trinity-legacy-install/etc/worldserver.conf`).
- JSON reports now include `login_only`, `world_auth`, `enum_characters`, and
  `player_login_verified`. Quest smoke reports additionally include
  `quest_smoke_passed`, target entry/spawn/map, `quest_ids_seen`,
  `quest_titles_seen`, and `quest_failure`.

The bot crate follows RustyCore's pinned `rust-toolchain.toml`. Use `cargo` for
standalone builds/tests of `tools/wow-test-bot`.

The wrapper normally builds the bot locally. To run a reproducible PR artifact
without invoking the local compiler, provide both the canonical executable path
and its verified hash:

```bash
WOW_BOT_EXEC=/absolute/path/to/wow-test-bot \
WOW_BOT_EXEC_SHA256=<sha256> \
./run_rustycore_login_smoke.sh
```

The wrapper rejects relative paths, symlinks, non-executable files, missing or
malformed hashes, and hash mismatches before starting the bot.
GitHub artifact downloads do not preserve executable mode; verify the published
hash first, then copy each binary into its immutable runtime path with
`install -m 0755` before invoking the wrapper.

The bot still supports the previous LFG path. Do not use LFG as the RustyCore
migration gate until the server-side LFG port is explicitly ready.

## Personal bank persistence smoke

Use the live low counter announced for the selected neutral banker; it is not
the persistent `world.creature.guid` spawn id:

```bash
WOW_BOT_BANK_SMOKE=1 \
WOW_BOT_BANK_RUNTIME_COUNTER=<live-banker-counter> \
./run_rustycore_login_smoke.sh
```

The pass requires all of these results: banker interaction opened, the fixture
item persisted from an empty backpack slot to an empty bank slot, a new full
authentication/login observed it there, withdrawal persisted back to the
backpack, and the second logout completed. Setup is restricted to `@bot.local`
accounts; cleanup removes only the generated item and restores the original
character position. `WOW_BOT_BANK_ITEM_ENTRY` (default `2589`) and
`WOW_BOT_BANK_TIMEOUT_SECS` are optional overrides.

## Void-storage atomic persistence smoke

The harness normally discovers the selected neutral vault keeper's live low
counter from the login object stream:

```bash
WOW_BOT_VOID_STORAGE_SMOKE=1 \
./run_rustycore_login_smoke.sh
```

The pass uses four complete authentications to prove unlock plus money/flag
persistence, deposit plus its new void-item identity, slot `0→5` persistence,
withdrawal into a bound inventory item, and a final empty void-store query.
Setup is restricted to an offline `@bot.local` character with empty void
storage and no matching fixture item. Cleanup removes the isolated item and
restores the original money, player flags, and position.
`WOW_BOT_VOID_STORAGE_ITEM_ENTRY` (default `2589`) and
`WOW_BOT_VOID_STORAGE_TIMEOUT_SECS` are optional overrides.
`WOW_BOT_VOID_STORAGE_RUNTIME_COUNTER` can pin the live counter for capture
work, but it is verified against the discovered ObjectGuid and cannot bypass
the SQL-position match.

## Innkeeper homebind persistence smoke

```bash
WOW_BOT_HOMEBIND_SMOKE=1 \
./run_rustycore_login_smoke.sh
```

The pass requires the triggered bind spell plus all three bind/gossip response
packets, a matching `character_homebind` row, and a second complete
authentication/login that observes the same persisted row. Setup is restricted
to `@bot.local` accounts and cleanup restores the original character position
and homebind exactly.
`WOW_BOT_HOMEBIND_RUNTIME_COUNTER` is an optional override; the current
default discovers the map-owned low counter from the login update stream.

## Direct inventory swap persistence smoke

```bash
WOW_BOT_INVENTORY_SWAP_SMOKE=1 \
./run_rustycore_login_smoke.sh
```

The pass requires two isolated items to exchange occupied backpack slots. Item
A has a permanent enchantment and a real random property; item B carries an
explicit all-zero 13-slot enchantment shape. The bot records A's complete
owner-visible `CREATE_OBJECT`, logs out over the preserved realm/instance
topology, and only then checks both items' committed locations and metadata,
matching C++'s logout-save lifecycle. Fresh authentication after the forward
and reverse exchanges must publish the identical block hash. Every phase must
observe `SMSG_LOGOUT_COMPLETE`, accepted only with C++'s empty body. The live
C++ and Rust issue-#20 contrast produced the same block SHA-256:
`25238a033be693b4969b9412f1666074e5d9be76c6db3b188e021a60b4feb2c8`.
Setup and cleanup are restricted to `@bot.local` accounts.
`WOW_BOT_INVENTORY_SWAP_ITEM_ENTRY_A/B` (defaults `2589`/`2592`) and
`WOW_BOT_INVENTORY_SWAP_TIMEOUT_SECS` are optional.

## Rested XP accrual and consumption smoke

Run the complete offline-accrual, kill-consumption, DB-persistence, and relog
round-trip with:

```bash
WOW_BOT_RESTED_XP_SMOKE=1 \
WOW_BOT_ACK_DISPOSABLE_RESTED_XP=1 \
./run_rustycore_login_smoke.sh
```

The wrapper converts `WOW_BOT_ACK_DISPOSABLE_RESTED_XP=1` into the mandatory
`--ack-disposable-rested-xp` CLI flag. The bot binary deliberately has no
environment-variable bypass for that acknowledgement, and the flag is rejected
unless rested-XP smoke is enabled.

The default fixture uses Mana Wyrm entry `15274`, simulates `86400` seconds
offline, and allows `120` seconds for the live protocol phase. The conservative
bound accommodates the current unarmed canonical-player damage boundary while
still failing closed on an invalid target or a stalled combat stream. It validates both
offline rates (wilderness and a resting location), then gives the character a
known rest pool and attacks a real nearby creature. A pass requires the
corresponding `SMSG_LOG_XP_GAIN`, its base/rested split, matching XP/rest values
in the character DB, and a fresh authentication/login that observes the same
persisted values and `restState`. The live bot does not yet decode the nested
ActivePlayer XP/RestInfo fields inside `SMSG_UPDATE_OBJECT`; their atomic mask
and values remain covered by focused packet/unit tests, not claimed as a live
wire assertion here.

Each rested-XP phase requests a normal logout, handles the stock C++ wilderness
countdown (including time-sync traffic), then closes both realm and instance
sockets and waits for a stable offline character row before reading
persistence. The bounded DB wait also covers C++'s 60-second raw socket-loss
session expiry if a runtime closes before `SMSG_LOGOUT_COMPLETE`. The workflow
does not assume that `Player::GiveXP` writes the database before character
save.

Useful overrides:

- `WOW_BOT_RESTED_XP_CREATURE_ENTRY` / `--rested-xp-creature-entry`: target
  creature template entry; default `15274`.
- `WOW_BOT_RESTED_XP_CREATURE_GUID` / `--rested-xp-creature-guid`: optional
  exact persistent `world.creature.guid` spawn identity.
- `WOW_BOT_RESTED_XP_RUNTIME_COUNTER` / `--rested-xp-runtime-counter`:
  optional live map-generated `ObjectGuid` low counter. It is fail-closed: the
  same counter must be discovered near the selected SQL spawn in that login's
  `SMSG_UPDATE_OBJECT` stream; it cannot bypass spawn discovery.
- `WOW_BOT_RESTED_XP_OFFLINE_SECS` / `--rested-xp-offline-secs`: simulated
  offline interval; default `86400`. It must fit `uint32` and be smaller than
  the current Unix timestamp.
- `WOW_BOT_RESTED_XP_TIMEOUT_SECS` / `--rested-xp-timeout`: live protocol
  timeout; default `120`. Natural-respawn cleanup uses the larger of this value
  and the persisted runtime `respawnTime` plus a 15-second tick grace. A pass
  requires observing the DB transition from a present timer to a stable absent
  row; absence alone is not treated as proof of respawn. The precheck accepts
  only SQL respawns from 30 through 600 seconds, and the observed wait has a
  fail-closed 900-second safety bound.

The GUID overrides are not normally required: the bot selects the requested
spawn near the fixture position and discovers its live counter in the login
`SMSG_UPDATE_OBJECT` stream. The wire GUID does not contain the persistent SQL
spawn id, so the harness links both identities fail-closed through entry, map,
and proximity to the selected spawn's SQL home position. Once it discovers the
live position, it first acknowledges active-mover initialization with
`CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE` (required by C++ visibility), then sends
a complete `CMSG_MOVE_HEARTBEAT` to stand one yard away
and face the target before `CMSG_ATTACK_SWING`. During the bounded combat wait,
it answers every periodic `SMSG_TIME_SYNC_REQUEST` with the C++-layout
`CMSG_TIME_SYNC_RESPONSE`. This mirrors a real client and keeps the session
active while a full-health target takes longer than the idle-session interval,
instead of silently ceasing to advance player auto-attacks.

Without an explicit trusted runtime counter, preflight also rejects another
same-entry/map SQL spawn whose movement radius overlaps the selected target's
matching sphere. This prevents a nearby wanderer from satisfying discovery
while cleanup watches the wrong persistent spawn.

This is a destructive **disposable-fixture-only** smoke, not a rollback-safe test
for a normal character. Its cleanup restores the explicitly recorded
`characters` fields used by the workflow (level/XP/rest flags and bonus,
logout marker, location, health/powers, kill counters, and played time). Because
preflight proves they start empty, cleanup also removes only this fixture's
deterministic login/save rows from `character_glyphs`, `character_reputation`,
`character_skills`, and `battlenet_account_transmog_illusions`. These rows are
the bounded defaults materialized by a stock C++ login/save on the disposable
fixture; an unexpected protected-table mutation is still left visible and
fails the next preflight. It also snapshots and exactly
restores `character_achievement` and `character_achievement_progress`, which a
real C++ kill can mutate, plus `character_trait_config` and
`character_trait_entry`, where C++ login/save can materialize missing
specialization defaults. It likewise preserves optional homebind, fishing, and
battleground rows, all game-account last-played rows, and Battle.net pet slots;
stock C++ can replace or materialize those during login/save. The smoke fails
closed unless `PlayerSave.Stats.MinLevel=0`, because enabling that diagnostic
table makes C++ rewrite `character_stats` on logout, and unless
`PlayerStart.AllSpells=0`, which prevents configuration-driven spell
materialization. Other
character/account/Battle.net tables remain outside that bounded restore.

The precheck therefore requires an `@bot.local` identity whose configured email
matches the Battle.net owner of the selected game account, with exactly one
character on its game account and exactly one game account on its Battle.net
identity, `characters.at_login = 0`, no Recruit-A-Friend or group membership,
and no active quest/objective/criteria state. It also rejects non-empty
high-risk state in `character_inventory`, pets, auras, spell cooldowns/charges,
skills, glyphs, talents, spells/favorites, action bars, reputation,
equipment/transmog sets, CUF profiles, corpses, tutorials, account instance
locks, guild membership, void storage, Battle.net pets, and Battle.net
collection tables. The target must have no on-kill
reputation and no pre-existing respawn row. These checks reduce collateral
mutation; they do not turn the bounded field restore into a complete database
backup. If the server creates rows in any other protected table during the
login/logout cycle, cleanup intentionally leaves them visible so the next
preflight fails instead of hiding a new side effect.

After the workflow, the harness waits for the server's normal runtime respawn
to remove the target's `respawn` row and verifies stable absence. It never
deletes that row manually. If the bounded wait expires, the smoke fails and
reports the remaining spawn/map/`respawnTime`; wait for the runtime respawn
before retrying. Do not interrupt cleanup, never point this mode at a player's
normal character, and inspect `rested_xp_failure` after any failure.

Default artifacts:

```text
/tmp/rustycore-bot-rested-xp-smoke.log
/tmp/rustycore-bot-rested-xp-smoke-report.json
```

The report summary exposes `rested_xp_smoke_passed`, both offline bonuses,
target entry/spawn/runtime ids, XP packet `amount`/`original`, DB XP/rest values
before and after consumption, `rested_xp_relog_verified`, and
`rested_xp_failure`.

## Default RustyCore smoke command

Use the wrapper:

```bash
./run_rustycore_login_smoke.sh
```

By default it runs one account:

```text
TESTBOT1@bot.local
```

It writes:

```text
/tmp/rustycore-bot-login-only.log
/tmp/rustycore-bot-login-only-report.json
```

The expected report shape for a pass is:

```json
{
  "login_only": true,
  "results": [
    {
      "account": "TESTBOT1@bot.local",
      "world_auth": true,
      "enum_characters": true,
      "player_login_verified": true,
      "join_result": null
    }
  ]
}
```

If no bot password is configured, the wrapper generates one in ignored
`tools/wow-test-bot/.env.local` and exports it for the run. By default it also
passes `--ensure-test-accounts`, which creates missing local `@bot.local`
BNet/game identities or validates an already complete identity. It does not
rewrite existing credentials, clear locks/bans, repair partial identities,
reassign characters or overwrite an online/realm-count mismatch. Existing
credentials and configured character ownership must match or the run fails.

Disable these local QA helpers with:

```bash
WOW_BOT_GENERATE_LOCAL_PASSWORD=0 \
WOW_BOT_ENSURE_TEST_ACCOUNTS=0 \
./run_rustycore_login_smoke.sh
```

## Useful overrides

```bash
WOW_BOT_PASSWORD='local-password' WOW_BOT_ACCOUNT=TESTBOT2@bot.local ./run_rustycore_login_smoke.sh

BNET_HOST=127.0.0.1 BNET_PORT=8081 \
WORLD_HOST=127.0.0.1 WORLD_PORT=8085 \
INSTANCE_HOST=127.0.0.1 REALM_ID=1 \
WOW_BOT_BUILD=54261 \
WOW_BOT_PASSWORD='local-password' \
./run_rustycore_login_smoke.sh
```

If the DB names or credentials differ from the runtime config, either point at
that config:

```bash
WOW_BOT_DB_CONF=/path/to/worldserver.conf \
WOW_BOT_PASSWORD='local-password' \
./run_rustycore_login_smoke.sh
```

or set explicit DB URLs through an ignored `tools/wow-test-bot/.env.local`.

## Quest / gossip smoke

Use this when QA needs to prove that a visible questgiver actually responds and
that the offered quest set matches class/race/level expectations:

```bash
WOW_BOT_QUEST_SMOKE=1 \
WOW_BOT_QUEST_CREATURE_ENTRY=15513 \
WOW_BOT_QUEST_EXPECT_ID=<hunter-training-quest-id> \
WOW_BOT_QUEST_FORBID_TITLE_CONTAINS='Mage' \
WOW_BOT_PASSWORD='local-password' \
./run_rustycore_login_smoke.sh
```

For deterministic accept-flow QA, let the bot prepare a test character before
login:

```bash
WOW_BOT_QUEST_SMOKE=1 \
WOW_BOT_QUEST_CREATURE_ENTRY=15278 \
WOW_BOT_QUEST_MAP_ID=530 \
WOW_BOT_QUEST_EXPECT_ID=9393 \
WOW_BOT_QUEST_RESET=1 \
WOW_BOT_QUEST_RELOCATE=1 \
WOW_BOT_QUEST_SET_RACE=10 \
WOW_BOT_QUEST_SET_CLASS=3 \
WOW_BOT_QUEST_SET_LEVEL=3 \
WOW_BOT_QUEST_ACCEPT=1 \
WOW_BOT_PASSWORD='local-password' \
./run_rustycore_login_smoke.sh
```

For objective load/save QA, seed an active quest with objective counters, then
force a real logout and compare `character_queststatus_objectives` after the
server save:

```bash
WOW_BOT_QUEST_SMOKE=1 \
WOW_BOT_QUEST_CREATURE_ENTRY=15278 \
WOW_BOT_QUEST_MAP_ID=530 \
WOW_BOT_QUEST_EXPECT_ID=9393 \
WOW_BOT_QUEST_OBJECTIVE_PERSIST=1 \
WOW_BOT_QUEST_OBJECTIVES=0:1 \
WOW_BOT_PASSWORD='local-password' \
./run_rustycore_login_smoke.sh
```

Useful quest overrides:

- `WOW_BOT_QUEST_CREATURE_ENTRY`: required creature template entry.
- `WOW_BOT_QUEST_CREATURE_GUID`: optional exact `world.creature.guid` spawn.
- `WOW_BOT_QUEST_GUID_COUNTER`: optional live `ObjectGuid` low counter. C++ `Creature::LoadFromDB` uses a map-generated lowguid for the live creature and keeps the DB spawn guid separately.
- `WOW_BOT_QUEST_MAP_ID`: optional map override used for GUID construction.
- `WOW_BOT_QUEST_EXPECT_ID`: require this quest id in list/details.
- `WOW_BOT_QUEST_FORBID_ID`: fail if this quest id is offered.
- `WOW_BOT_QUEST_FORBID_TITLE_CONTAINS`: fail if any offered title contains this
  text, case-insensitive.
- `WOW_BOT_QUEST_QUERY_DETAILS=0`: skip the non-mutating
  `CMSG_QUEST_GIVER_QUERY_QUEST` details probe.
- `WOW_BOT_QUEST_RESET=1`: remove the expected quest from the selected bot
  character's active/rewarded quest tables before login.
- `WOW_BOT_QUEST_RELOCATE=1`: move the selected bot character near the resolved
  creature spawn before login.
- `WOW_BOT_QUEST_SET_LEVEL=<1-80>`: set the selected bot character level before
  login so class/race/level filters are tested deterministically.
- `WOW_BOT_QUEST_SET_RACE=<id>`: set the selected bot character race before
  login for race-gated quest QA.
- `WOW_BOT_QUEST_SET_CLASS=<id>`: set the selected bot character class before
  login for class-gated quest QA.
- `WOW_BOT_QUEST_ACCEPT=1`: send `CMSG_QUEST_GIVER_ACCEPT_QUEST` after details
  and verify the quest persisted in `character_queststatus`.
- `WOW_BOT_QUEST_OBJECTIVE_PERSIST=1`: seed the expected quest and objective
  rows, logout, and verify `character_queststatus_objectives` survived the
  server save.
- `WOW_BOT_QUEST_OBJECTIVES=<storage:data,...>`: objective rows to seed for
  persistence QA, using C++ `QuestObjective::StorageIndex` values.
- `WOW_BOT_QUEST_OBJECTIVE_STATUS=<n>`: optional quest status for the seeded
  `character_queststatus` row; defaults to `3` (incomplete).
- `WOW_BOT_ENSURE_TEST_ACCOUNTS=0`: skip the default local `@bot.local`
  account/password bootstrap.
- `WOW_BOT_ALLOW_NONLOCAL_ACCOUNT_BOOTSTRAP=1`: allow
  `--ensure-test-accounts` to touch non-`@bot.local` accounts. Keep this off for
  normal QA.

When no exact spawn guid is provided, the bot uses the selected character's
saved map/position and picks the nearest `world.creature` row for the requested
entry. The character must still be close enough in-game for RustyCore's
interaction-distance checks.

For DB-spawned creatures, `world.creature.guid` is the persistent spawn
identity while the live `ObjectGuid` low counter is map-generated. Quest and
bank mode therefore accepts an explicit runtime counter instead of pretending
the two identities are interchangeable.

## Known notes

- The bot updates `account.session_key_bnet` for the selected test account.
- `config.example.json` intentionally keeps passwords blank. Use env overrides
  or an ignored local `config.json`; never commit real bot passwords.
- `tools/wow-test-bot/.env.local` is ignored and is the right place for local
  DB URL overrides or bot passwords when needed.
- The current RustyCore BNet server does not expose the old C++ bot-only
  `/login/srp/` route. The bot falls back to `/bnetserver/login/`, then writes
  the generated world key into the auth DB for the world handshake.
- `SMSG_CONNECT_TO` / instance socket is handled by the current bot code; older
  C++ notes saying realm-socket-only are stale for this tree.
- Keep raw tickets/session keys out of chat and commit messages. Use the report
  booleans and sanitized log grep lines for status.
