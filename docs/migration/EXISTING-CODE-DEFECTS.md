# RustyCore — Defects in EXISTING (already-developed) code

**Date:** 2026-07-22 · **Base:** `3.4.3` @ `55719eb4` plus issue #20 closeout QA.

This is the adversarial audit the user asked for: **not what's missing, but what's wrong
in what already exists.** An 8-agent parallel pass tried to *break* the capabilities STATE.md
labels WORKS, contrasting each against C++ (`/home/server/woltk-trinity-legacy`).

**Reliability note:** these are agent findings; each carries `file:line` + a C++ ref, but
they are **leads to verify, not proven verdicts**. Two are explicitly contested between
agents (marked ⚠VERIFY). Signedness divergences (i32 vs u32) are rated **LOW** because they
only differ for values >2³¹ that never occur in normal play (stack counts, durability) —
identical bytes otherwise. Severity reflects my judgment after that filter.

The scoped **D-C1…D-C9 CRIT integrity track is closed** with its recorded evidence.
The HIGH/MED entries below retain historical diagnoses; their current existence and
severity require source contrast or reproduction. Sending a packet and mutating a
database is not by itself full gameplay parity.

## Current allocation — 2026-09-11, #748 / #49

The master [PORT_PLAN.md](PORT_PLAN.md) allocates every open issue at integration
`5d8c079a06b587c060c1c6e1c06bedb73c4339d0`. This reconciliation changes planning,
not the dates, source identities or execution evidence of the findings below.
Closed #578/#585/#587/#588/#589/#718/#722/#737 have changed relevant consumers;
an old unchecked row is not an instruction to implement its original diagnosis.

| Finding family | Current review/implementation owner |
| --- | --- |
| Melee, damage/heal, offhand/haste and threat modifiers | #29/#31 and the affected aura/proc/effect consumers #32/#33/#34; effective item stats #61. |
| Quest credit, timed/breadcrumb and accept/reward participants | Consolidated #41 (receives #58/#59), with loot #55 and area #56. Preserve the integrated #718 transaction. |
| Aura persistence, broader save/recovery and silent-save diagnoses | #32 for aura semantics; #584 for remaining core save/lifetime contracts, #54 admission, #45 instances, #47 real load/recovery acceptance. Recheck #585 evidence before declaring a residual. |
| Trainer/acquisition and effective skill data | Reuse #587; #524 owns the verified startup-order difference. A newly reproduced acquisition defect gets a bounded owner through #49, not automatic reopening of #587. |
| Group lifecycle/persistence/fanout | #51 for the complete gameplay operation; #743 for state-bearing command delivery/reconciliation. |
| Movement, visibility, entry and transfer presentation | #63 and #12, reusing #588. #12 also revalidates the dated cinematic-catalog/hotfix-routing findings in the entry/transition presentation contract. |
| Target player-name identity | #486, whose current adapter still loads a full character row and whose handler still uses querying-session account IDs. |
| Vendor/buyback races and item persistence | #584 retains represented inventory-operation integrity disposition; recheck #737 and existing transaction/fence evidence before selecting a repair. Wider unrepresented behavior stays in #48/L12. |
| Rest arithmetic, talent reset/load, glyph selection and other long-tail progression | #48/L18/L26; promote a reproduced defect into its affected Part-1 operation when it invalidates that operation's acceptance. |
| Lower-priority protocol/value and source-reference findings | #48/L2/L22/L26 and #65, with an exact current consumer before implementation. |

These are responsibility routes, not automatic hard dependencies between every listed
issue. A verified integrity failure takes priority in its affected operation. No row is
bulk-closed, retested or reclassified as parity-proven by this planning review.

## Later verified open findings

- [x] **2026-09-30, live: a creature a player attacks evades on the next aggro tick, and the
  evade cancels the player's attack — so the player never swings.** Both halves repaired;
  closed 2026-10-01, and kept here with the evidence that produced it. Root cause traced
  end to end on a running server with a real client session (tools/wow-test-bot
  `--melee-smoke`), reproduced against a critter (entry 721) and a hostile creature
  (entry 299), and in **both** tick-owner configurations.

  The chain, each step observed:

  1. The request is accepted and the state is written to the right object. At the end of
     `start_player_attack_like_cpp` (`session/combat/melee.rs:728`) the trace reports
     `outcome=NewTarget{previous:None}`, `residence=MapKey{0,0}` and the target read back
     through **both** routes the code uses: `attacking_via_handle=Some(Creature …)` and
     `attacking_via_map=Some(Creature …)`. There is one canonical Player and it has the
     victim. (An earlier version of this entry claimed two Player instances; that is
     disproven and corrected.)
  2. The creature only exists in the **legacy** runtime. The canonical creature scan
     reports `canonical_scan_ran=true canonical_found=0` while the legacy grid holds 29
     candidates. `begin_canonical_player_combat_ref_like_cpp`
     (`session/combat/state.rs:212`) therefore applies its combat reference on a map that
     does not contain the victim, and the legacy creature receives only
     `enter_combat` → `enter_ai_combat` (`wow-entities/src/creature/ops_1.rs:895`), which
     sets AI state, combat target and `attacking` — **no threat reference**.
  3. On the next aggro tick the legacy threat update finds no usable hostile for that
     creature and returns `LegacyCreatureThreatUpdateLikeCpp::Evade`
     (`session/legacy_runtime/creature_threat.rs:201,239,257`), which sets `UnitState::EVADE`
     and resets its combat.
  4. Evade emits one `CreatureAttackStopLikeCppCommand` per participant
     (`creature_aggro_tick.rs:595`). Observed exactly once, for our pair:
     `creature_combat_stop_applied attacker_guid=Creature[721 #48] victim_guid=Player[#5]`.
  5. Applying it runs `apply_creature_combat_stop` (`wow-map/src/map/runtime.rs:437`), whose
     Player-victim branch does `set_attacking(None)` and purges the player's combat
     reference. That mapping is faithful — C++ `Unit::CombatStop` → `RemoveAllAttackers` →
     each attacker's `AttackStop` — so the player's attack is cancelled 13-17 ms after it
     was accepted.
  6. The runtime phase that would swing then reads the same Player and sees
     `has_combat=false` with no `attacking()`, while **`selection` still holds the
     creature** — the fingerprint of step 5 rather than of `attack_stop_like_cpp`, which
     would also have cleared the selection. Confirmed by tracing the three session paths
     that could clear it (`run_combat_tick`'s vanish branch, `combat_stop_like_cpp`,
     `stop_player_attack_like_cpp`): **none fires**.

  Both tick owners fail for this one reason: with
  `RustyCore.LegacyCreatureGlobalRuntime = 1` the global player-melee phase reports
  `victims_resolved=0 attacker_unavailable=0` every 10 ms tick; with `= 0` nothing else
  clears the target and the session's own `run_combat_tick` still publishes no swing in a
  60-second engagement at 4 yards.

  **Repaired for the player-initiated case on 2026-10-01.** The source of step 2 was
  RustyCore's own addition: `handle_attack_swing` put the victim into AI combat, which C++
  `Unit::Attack` does not do for a player attacker — it records the attacker in the victim's
  set (already done here through `add_attacker_like_cpp`) and reaches `EngageWithTarget`
  only inside `if (creature && !IsControlledByPlayer())` (`Unit.cpp:6254-6256`). The victim
  engages when damage lands. A first candidate repair — adding a zero threat reference — was
  **rejected** after reading that code: it would have invented C++ behaviour to keep an
  unfaithful combat entry alive. Removing the entry is the faithful fix, and it is what made
  the loop work live: `player_landed=1 (10 damage)`, `SMSG_ATTACK_STOP reports the target
  dead`, `--melee-smoke` exit 0 against entry 721.

  **Repaired on 2026-10-01 — the second half of the same chain, and both remaining
  symptoms with it.** The evade that kept cancelling the attack was traced to one
  unfaithful clause, not to chase or to a missing damage hook. Order captured live with
  `RUSTYCORE_PLAYER_MELEE_TRACE=1` plus a temporary backtrace on
  `CombatSubsystem::set_attacking` and on `WorldCreature::enter_combat`:

  1. `22:50:02.416 attack_accepted` — the player attacks creature 299.
  2. `22:50:02.449` — `apply_player_melee_to_legacy_creature_like_cpp`
     (`legacy_runtime/creature_melee_tick.rs:289`) engages the creature. So player damage
     *does* engage its victim; the earlier "nothing engages a creature when player damage
     lands" reading was wrong, and it is withdrawn here.
  3. `22:50:02.492` — the aggro tick's threat update takes every reference offline and
     evades. A per-participant diagnostic printed
     `targetable=true accessible=true visibility=Allowed leash=Allowed` and
     **`hostile=false`**: the only failing clause was hostility.
  4. `22:50:02.494` — the evade's `CombatStop` clears the player's `attacking`, which is
     faithful: C++ `CreatureAI::_EnterEvadeMode` (`AI/CreatureAI.cpp:315`) calls
     `Unit::CombatStop` → `RemoveAllAttackers` (`Unit.cpp:6377`), and that calls
     `AttackStop` on every attacker. The propagation was never the defect.

  `ThreatReference::ShouldBeOffline` (`Combat/ThreatManager.cpp:99-108`) never re-asks
  whether a participant is hostile. It asks `Creature::_IsTargetAcceptable`, whose decisive
  clause is `IsEngagedBy(target) || IsHostileTo(target)` (`Creature.cpp:2717`), and
  `Unit::IsEngagedBy` (`Unit.h:1025`) reads the threat list through `Unit::IsThreatenedBy`
  (`:1055`) with **`includeOffline = true`** — so a reference that merely exists is
  acceptance on its own. Hostility gates *starting* a fight, not keeping one. The Rust
  eligibility set demanded hostility for every participant, so a creature a player attacked
  dropped the player on the next tick unless the factions were hostile, evaded, and
  cancelled the swing.

  The repair adds `legacy_creature_candidate_is_acceptable_target_like_cpp` (and its
  snapshot form) as the single port of `_IsTargetAcceptable`, used by both the threat update
  and the aggro gate, and splits the old boolean into
  `WorldObject::GetFactionReactionTo` (`Entities/Object/Object.cpp:2855`) so `IsHostileTo`
  and `IsFriendlyTo` come from one reaction instead of one conflated predicate — the
  friendly clause at `Creature.cpp:2702` needs the distinction. Live proof on the same
  spawn, one run: `attack_start=true player_landed=4 (42 damage) creature_landed=4
  (4 damage) death=true xp=50`, `SMSG_ATTACK_STOP reports the target dead`,
  `SMSG_LOG_XP_GAIN 50 XP`, `--melee-smoke` exit 0. The creature now retaliates, dies, and
  pays experience.

- [x] **2026-10-01, live: a released spirit could resurrect instantly, anywhere, with no
  corpse at all — and the corpse it left behind had no owner.** Found because combat now
  kills the QA character routinely. `CMSG_RECLAIM_CORPSE` was a represented slice: it
  cleared the ghost flag and restored half health after checking only alive/ghost, skipping
  the four gates C++ `WorldSession::HandleReclaimCorpse`
  (`Handlers/MiscHandler.cpp:435-464`) applies — arena, a live corpse (`:449-450`), the
  reclaim delay (`:452-454`) and `CORPSE_RECLAIM_RADIUS` (`:456-457`) — and never reaching
  `SpawnCorpseBones` (`:463`), so the same corpse stayed reclaimable forever. Underneath,
  `create_player_corpse_on_map_like_cpp` never called `set_owner_guid`, where C++
  `Corpse::Create(guidlow, owner)` stamps it (`Entities/Corpse/Corpse.cpp:84-89`) and
  `Map::GetCorpseByPlayer` keys `_corpsesByPlayer` on exactly that field
  (`Maps/Map.cpp:3714`): the corpse existed and nothing starting from the dead player could
  find it. Repaired with the map-owned `corpse_by_player_like_cpp` and
  `convert_corpse_to_bones_like_cpp`, a `delete_corpse_like_cpp` on the corpse persistence
  port for the `Corpse::DeleteFromDB` transaction C++ commits inside the conversion, and the
  delay arithmetic of `Player::GetCorpseReclaimDelay` (`Player.cpp:25297-25312`). Live proof,
  six consecutive `--death-smoke` runs: release writes the corpse row and teleports the
  ghost, the corpse run returns, the reclaim is refused while the delay counts down
  27/22/17/11/6/1 and then takes, the row is gone, and a clean logout saves `health = 20`
  of 40 with no ghost flag. Known boundary, written on the code: the arena refusal is not
  ported because arenas are not represented and the battleground flag must not stand in for
  it, and `m_deathExpireTime` is reported unset because its only C++ writer lives in
  `Player::KillPlayer` (`:4327`), which has no Rust equivalent — unset is what C++ computes
  for a death older than five minutes, the first 30-second step.

- [x] **2026-10-01: the at-war reputation flag was treated as the hostility decision
  instead of a cap.** Found while splitting the reaction above.
  `WorldObject::GetFactionReactionTo` (`Entities/Object/Object.cpp:2880-2885`) reads the
  player's rank for the creature's faction and caps it at `REP_NEUTRAL` **only when the
  player is at war**. The Rust branch instead returned "not hostile" for every faction the
  player was not at war with. For hostility alone the two read the same on standings
  TrinityCore produces, because `ReputationMgr::SetReputation` declares war when a rank
  drops to hostile, but the shortcut cannot express a friendly reaction at all, which
  `Creature::_IsTargetAcceptable` needs. Corrected to the C++ shape, and the test that
  asserted the shortcut (`..._rejects_reputation_without_at_war_like_cpp`) was rewritten
  against the source as `..._caps_an_at_war_reaction_at_neutral_like_cpp`. Recorded
  separately because it is a behaviour change, not part of the melee repair.

- **2026-09-30, live: a player sees nothing until it acknowledges its active mover — this is
  C++ behaviour, recorded because it looks like a visibility defect.** While investigating the
  above, the first `--melee-smoke` runs found `candidate_creatures=0` with 22 creatures within
  100 yards, and `RUSTYCORE_CREATURE_VIS_TRACE=1` showed the scan finding 29 candidates in the
  legacy grid, 22 in range and **0 surviving** `can_see_or_detect_unit_like_cpp`, with
  `is_in_map=true` and `in_same_phase=true`. The cause is faithful:
  `Player::CanNeverSee` (`Entities/Player/Player.cpp:23214-23218`) hides every object from a
  player that lacks `PLAYER_LOCAL_FLAG_OVERRIDE_TRANSPORT_SERVER_TIME`, which the server sets
  when the client sends `CMSG_MOVE_INIT_ACTIVE_MOVER_COMPLETE`. RustyCore implements it at
  `session/mod.rs:9530`. No defect: a harness that skips that packet simply sees an empty world.
  The existing bot workflows already send it; `--melee-smoke` now does too.

- **2026-09-11, #743 group removal — the member's own state clear can be dropped.**
  Source-verified on `6aeca244`. When a member is kicked or the group disbands,
  the acting session mutates the registry and then asks the affected member's
  session to clear its own Player state:
  `handlers/group/ops_1.rs:632` sends `SessionCommand::ApplyGroupRemovalLikeCpp`
  through `PlayerDirectory::try_send_current_command`
  (`session/directory.rs:1753`), a `try_send` on a bounded channel that returns
  `PlayerDirectorySendError::Full` when the target queue is full. The production
  queue is `flume::bounded(256)` (`session/mod.rs:7726`) and the result is
  discarded with `let _ =`; 14 of the 23 production call sites discard it, while
  `session/admission.rs:436` handles it, so the codebase is already inconsistent
  about whether a dropped command matters. Nothing reconciles afterwards:
  `sync_player_registry_state_like_cpp` (`session/mod.rs:12431`) pushes session
  state *to* the registry and never re-derives the Player's group from it, and
  although `set_owned_player_group_like_cpp` validates against the registry when
  setting, some consumers still use the Player snapshot. The 2026-09-11 review
  at `5d8c079a` corrects the earlier blanket claim: some social/chat readers
  revalidate membership, while tap/instance readers need explicit coherence.
  A lost removal can leave the registry and Player disagreeing. C++
  `Group::RemoveMember` clears the linked Player reference directly; Rust's
  cached DTO needs equivalent application/reconciliation guarantees.
  Severity is bounded by the trigger, which needs a saturated or stalled session
  loop. This is a source-verified mechanism, not a live reproduction; no live
  capture or runtime QA was run for it. **Repaired and locally accepted at
  `9e6767bb` under #743:** a group state change that cannot be handed to its member
  records a delivery obligation, and the member converges on `GroupRegistry` in a
  dedicated driver phase; the tap and instance readers named above now resolve
  through the authority. No live runtime or DB evidence exists for that repair.

- **2026-09-05, #578 quest dialog — repeatable turn-in markers reversed.**
  Source-verified on `e478ac5d`: in `handlers/quest/eligibility.rs`,
  `get_represented_quest_giver_status_like_cpp` selects TRIVIAL_REPEATABLE_TURNIN
  when `represented_quest_is_trivial_like_cpp` is true and REPEATABLE_TURNIN
  otherwise. C++ `Player.cpp:15742-15748` selects RepeatableTurnin above the
  quest-level-plus-hide-difference threshold and TrivialRepeatableTurnin otherwise.
  This is a static branch discrepancy, not a live-client reproduction. The pure
  dialog-classification extraction leaves this eligibility branch unchanged;
  correcting it needs a separate behavior change and packet-status evidence.

- **2026-09-05, #578 cinematic catalog — production wiring absent.**
  Source-audited on `a3b03e65`: `WorldSession::cinematic_sequences_store` starts
  as None; `set_cinematic_sequences_store` is called only by tests. No
  CinematicSequences load or installation exists in `world-server` startup.
  `send_represented_cinematic_start_like_cpp` sends TriggerCinematic before
  optionally initializing native Player camera IDs; consequently production
  emits the trigger without this camera-state initialization. C++ loads
  `sCinematicSequencesStore` in `DB2Stores.cpp:106,681`, and
  `Player.cpp:6178-6185` starts CinematicMgr after sending the packet.
  `session/tests/cinematic_catalog.rs` characterizes absent/present catalog
  against a canonical Player. This is not a live-client reproduction or a
  claim that all cinematic behavior is missing. Startup wiring would change
  behavior and must be a distinct correction from the #578 catalog ownership
  move; both opening cinematics and GameObject cameras must consume the same
  process-owned catalog. Full fly-by camera/runtime parity remains unproven.

- **2026-09-05, #578 rest consumption — percentage arithmetic differs.**
  Verified on `b98903e8`: Session's rested-consumption helper uses signed `i64`
  multiplication/division (truncation toward zero) then clamps the final loss
  into `u32`. C++ `RestMgr.cpp:125-138` calls `AddPct(uint32&, ...)`;
  `src/common/Utilities/Util.h:71-87` computes the percentage through `float`
  and converts the term to `T` before addition. Negative/out-of-range conversion
  and float precision boundaries are not equivalent to the represented Rust
  arithmetic. The subsequent Player-owner refactor preserves the Rust formula;
  it is not a parity correction. Focused tests pin positive, negative, extreme
  and fractional truncation behavior, including 3 XP with -50% consuming 2.
  Any arithmetic correction requires separate behavioral evidence and validation;
  no client-capture or full RestMgr parity is claimed here.

- **2026-09-05, #578 save-owner read — save projection and writeback debt.**
  Verified on `b813d262`: `current_player_save_to_db_snapshot_like_cpp` uses
  Session's staged level/map, although C++ `Player.cpp:19480-19514` reads Player
  directly and substitutes only the persisted teleport destination. Rust also
  projects dead health differently by residence: active reads force zero when
  `is_alive()` is false, while detached reads clamp health to max without that
  gate; C++ writes `GetHealth()` (`19557`). The existing
  `sync_session_from_save_to_db_snapshot_like_cpp` then reapplies position, level,
  XP, money and health, including derived side effects, before the save request.
  The single-owner read refactor preserves these rules and does not retire this
  writeback bridge. Identity migration and separation of save-only destination
  from runtime mutation remain explicit #578 work, not approved parity or a
  deferred #153 exception. No reproduced live-client failure is asserted.
  **Local correction after `720b2519`:** the full-save recording-port regression
  reproduced relocation to the pending near destination even on definite
  rollback. The writeback method is now deleted; the request uses the captured
  header without replaying setters. Applied/Failed/Unknown outcomes are covered
  by the new regression. The staged level/map and residence-specific health
  projection findings remain open, as does live save/teleport acceptance.

- **2026-09-05, #578 talent-reset cost ownership — arithmetic boundary discrepancy.**
  Verified on `95cb0a34`: Rust's `next_reset_talents_cost_like_cpp` uses
  saturating time subtraction and fee addition, and a widened signed monthly
  reduction. C++ `Player.cpp:3472-3503` uses unsigned subtraction followed by
  signed narrowing. Normal reset history follows the same schedule, but future
  reset timestamps and extreme stored costs are not proven equivalent. The
  ownership move preserves Rust arithmetic; reconciling abnormal persisted
  values requires a separate behavior analysis, not an unannounced refactor
  change. No live-client failure is asserted.

- **2026-09-05, #578 talent-tab extraction — login applies extra tab/class gates.**
  On pre-slice `194f9d1b`, `load_represented_talent_row_like_cpp` validates a
  TalentTab row and class mask for both login and learning. C++
  `Player.cpp:26036-26058` applies these gates in `LearnTalent`, whereas
  `_LoadTalents` (`26623-26633`) delegates directly to `AddTalent`
  (`2644-2692`), which does not perform a tab/class lookup. The Rust login
  filtering is preserved by the catalog refactor, not claimed as C++ parity.
  Any behavior change needs separate analysis of persisted invalid rows and
  client/runtime effects; no observed client failure is asserted here.

- **2026-09-05, #578 glyph catalog extraction — represented glyph loading differs from C++.**
  Verified on pre-slice `b4d407b9`: `load_represented_glyph_row_like_cpp` in
  `crates/wow-world/src/session/mod.rs` skips catalog validation for glyph ID zero
  and writes `glyph_groups[talent_group][glyph_slot]`. C++ `Player.cpp:26573-26598`
  checks `sGlyphPropertiesStore.LookupEntry(glyphId)` even for zero and calls
  `SetGlyph`; `Player.cpp:25477-25481` writes to `GetActiveTalentGroup()`.
  The represented zero-row clearing and row-selected group remain unchanged in
  this ownership refactor. The active/detached borrowed-catalog regression retains
  zero clearing explicitly. Whether the legacy group selection is itself a defect
  needs separate client/persistence evidence before changing either policy. This
  is a verified source discrepancy, not a claim of a reproduced client failure.

- **2026-09-05, #578 catalog extraction — HotfixConnect uses the primary socket.**
  Confirmed against pre-slice `13c984a6`: `handle_hotfix_request` in
  `crates/wow-world/src/handlers/character/account.rs` calls generic `send_packet`, which
  writes to the primary channel (`session/mod.rs`). `wow-session/src/lib.rs`
  `poll_instance_link` replaces that channel with the instance writer after ConnectTo.
  C++ `Opcodes.cpp:1566` routes `SMSG_HOTFIX_CONNECT` exclusively over Realm.
  Before ConnectTo the primary is Realm and delivery agrees. The new shared-catalog
  dispatch test reproduces primary delivery with a parked Realm channel, including an
  empty response. This behavior is deliberately preserved by the structural extraction;
  a separate response-routing correction needs byte/routing regression and capture
  evidence. No live client failure or affected-client frequency is claimed.

## Later verified Rust-port repairs

- **2026-09-05, #578 optimized runtime QA — Map insertion vanished in release.**
  `Map::insert_map_object_record` performed `entity_world.insert(record)` inside `debug_assert!`.
  With debug assertions disabled, the record was never inserted; the derived indexes could still
  be updated and a Player lifetime could claim Active residence without a stored Player. This
  affects all map-record kinds, not only login. The insertion now executes unconditionally and
  only the displaced-record invariant is debug-only. C++ `Map::AddPlayerToMap`
  (`Map.cpp:427-445`) performs insertion independently of `ASSERT`. The production-linked login
  test now also reaches EquipmentInventory after map selection and interleaved map ticks.
  On the old code it passes in dev and fails in release; the missing-manager rejection and
  pre-map hydration tests pass in both. No ownership duplication, SQL, opcode or new clock is
  introduced. Post-fix validation and installed QA are recorded in the Session checkpoint.

- **2026-09-04, #578 runtime QA — initial Player construction depended on its own inventory.**
  Production login reached the instance socket, then kicked with `canonical Player mail owner
  disappeared`. `build_initial_player_for_owner_like_cpp` called presentation hydration, which
  queried canonical inventory before the new Player handle existed. Unit fixtures supplied a
  Session-side inventory and masked the cycle. Initial equipment hydration is now fixture-only;
  production keeps Player's initial empty equipment until the existing inventory load. C++
  constructs Player in `CharacterHandler.cpp:1065-1070`, establishes the session Player at
  `Player.cpp:17378`, and loads inventory/mail at `17748/17759`. No SQL or packet layout changes.
  The production-linked `production_login_player_owner` regression fails on the old code and
  passes with the fix; its missing-manager case rejects continuation. It stops at the PetStable
  read after mail/scalar hydration and does not claim a complete login. Live QA is recorded in
  the Session checkpoint separately.

- **2026-09-04, #578 runtime QA — nullable LFG hotfix text aborted startup.** The checked
  candidate rejected `LFGDungeons.Description` SQL NULL; the local positive-build batch has
  99 rows, two with NULL descriptions. C++ `Field::GetString` (`Field.cpp:118-126`) returns
  empty text for NULL. `DB2DatabaseLoader.cpp:121-132,275-287` preserves an existing localized
  string for an empty hotfix; `DB2LoadInfo.h:3365-3372` classifies both Name and Description
  as `FT_STRING`. The MariaDB adapter now distinguishes a valid nullable text value from a
  missing/mistyped column, and `wow-data::LfgDungeonsStore` preserves previous text while
  applying numeric fields. Focused tests cover missing rows, null/empty/nonempty SQL text,
  wrong types and missing columns, previous/new IDs and successive overlays; the explicit
  read-only MariaDB regression passed. This is a behavior correction separate from the
  Session capability extraction. Custom-row batching and other locale coverage remain outside
  this bounded fix; full startup/login acceptance is recorded separately.

## Bounded legacy repairs accepted during the port

- [x] **Issue #161 — battle-pet trainer purchases no longer strand the charge across the
  Character/Login commit window.** Legacy `Trainer::TeachSpell` charges money in memory and
  `BattlePetMgr::AddPet` builds the pet in memory; both sides persist only at the next
  `Player::SaveToDB`, which commits Character DB first and Login DB second
  (`Player.cpp:19336-19344`; money via `CHAR_UPD_CHARACTER` at `Player.cpp:19498-19505`, pet via
  `LOGIN_INS_BATTLE_PETS` at `BattlePetMgr.cpp:340-364`). A crash or failed Login commit between
  the two keeps the charge and loses the pet, and `BattlePetMgr::SaveToDB` clears
  `SaveInfo = BATTLE_PET_UNCHANGED` when statements are *appended*, before the commit result is
  known (`BattlePetMgr.cpp:377`), so the insert is never retried and the loss is silent; the
  dependent learned spell is intentionally never persisted (`Player.cpp:20437-20448`), leaving no
  proof of purchase. Rust instead records a durable saga command in the same Character DB
  transaction that deducts the guarded money, applies it once through the #160 account owner
  (whose Login DB transaction writes pet + receipt together under the account fence), queues
  publication only after the pet is durable, records the publication marker after enqueue, then
  completes the command, and refunds terminal failures exactly once; login recovery
  converges any interrupted command. Focused fault-injection tests distinguish this repair from
  both the legacy loss (charged without pet) and a speculative rewrite (no distributed
  transaction, no second journal owner): every crash boundary converges to either paid+pet or
  refunded+no-pet. Packet enqueue attempts are recoverable and may repeat after a crash between
  enqueue and marker; actual delivery remains best-effort without a client ACK. This is preferable
  to consuming the sole durable recovery signal before attempting the notification.
- [x] **Issue #159 — keep arena/battleground spell disables contextual.** Legacy
  `DisableMgr::IsDisabledFor` checks the arena and battleground flags, but when neither context
  matches and no map/area flag follows it falls through to the unconditional global-disable
  return (`DisableMgr.cpp:285-345`). Rust treats arena, battleground, map and area as location
  scopes: a scoped row disables the spell only when at least one declared scope matches. Focused
  tests pin normal-world rejection of the legacy fallthrough and positive arena/battleground
  matches.
- [x] **Issue #163 — rebuild skill indexes after final hotfix removals.** Legacy C++ builds
  selected `SkillLineAbility` / `SkillRaceClassInfo` derived indexes before
  `DB2Manager::LoadHotfixData` performs its final `RecordRemoved` pass
  (`DB2Stores.cpp:1328-1334,1539-1607`). That can leave a removed record reachable through a
  stale index. Rust composes WDC4 → official SQL → custom SQL → final removal first, then rebuilds
  every acquisition index from the surviving rows in ascending record-ID order. Focused fixtures
  distinguish this repair from both the stale C++ outcome and an unrelated rewrite.
- [x] **Issue #163 — an empty world `spell_learn_spell` table no longer erases canonical
  learning edges.** Legacy `SpellMgr::LoadSpellLearnSpells` returns before scanning
  `SpellEffect` and `SpellLearnSpell.db2` when the custom world query has no rows
  (`SpellMgr.cpp:990-1135`). Rust treats that result as zero custom rows and still builds the
  canonical graph. The loader test pins both effective edge families with an empty SQL input.
- [x] **Issue #163 — reject lossy acquisition narrowing.** Legacy
  `SpellMgr::LoadSpellLearnSkills` implicitly narrows effect-derived skill and step values to
  `uint16`, and DB-backed difficulty values to the `uint8` `Difficulty` enum
  (`SpellMgr.cpp:947-988,2730-2940`). Rust preserves checked source values in the immutable
  acquisition catalog and omits an unrepresentable compatibility node instead of authorizing a
  wrapped identifier. It also rejects an `EffectBasePoints` value whose C++ `float` round-trip
  would fall outside `int32`, rather than inheriting an undefined C++ cast or Rust saturation.
  Positive and negative fixtures pin the first-final-effect rule.
- [x] **Issue #163 — ranged learn-skill tiers are explicit instead of restart-random.** Legacy
  `SpellMgr::LoadSpellLearnSkills` calls `SpellEffectInfo::CalcValue()` once during startup
  (`SpellMgr.cpp:947-988`, `SpellInfo.cpp:495-559`), so a custom `SPELL_EFFECT_SKILL` with
  variance or ranged `DieSides` can select a different skill tier, tier maximum and durable
  player state after a restart whenever its rounded result domain has multiple values. The audited
  effective 3.4.3 data has 98 such effects and all are deterministic (`DieSides = 1`, zero
  variance/coefficient), so Rust preserves every official node and step. For custom/future
  ambiguous metadata it retains the complete checked value domain—including `frand`'s exclusive
  upper endpoint—and publishes a typed indeterminate lookup instead of silently treating the
  spell as having no learn-skill effect or inventing a minimum/maximum/average. The pure
  acquisition planner in #164 must consume that lookup and fail before mutation.
- [x] **Issue #163 — malformed effective rank graphs cannot hang startup or masquerade as
  unranked spells.** Legacy `SpellMgr::LoadSpellRanks` follows `SupercedesSpell` without cycle
  detection (`SpellMgr.cpp:812-902`); a custom/hotfix graph with a reachable cycle can loop
  forever, while merges and stale predecessor bookkeeping can construct incoherent chains. Rust
  builds a rank-specific projection from every final effective `SkillLineAbility` identity before
  hydrating unrelated acquisition fields, so an invalid race/skill mask neither erases a valid
  rank edge nor hides an invalid rank endpoint. Final hotfix removals still win. Rust then resolves
  valid and indeterminate candidates through one RecordID-ordered, last-wins authority per
  predecessor, rejects the complete ambiguous component for self-loops, cycles, multiple
  predecessors, ranks outside `uint8`, or unrepresentable endpoints, and retains a tri-state
  diagnostic lookup so later acquisition planning fails closed. A later valid candidate can
  repair an earlier malformed candidate for the same predecessor. If a representable endpoint is
  absent from exact spell authority, Rust skips the row just as C++'s paired `GetSpellInfo` gate
  does. Only a row with neither endpoint representable in C++'s `int32` source domain makes the
  rank projection globally indeterminate rather than inventing `Unranked`.
- [x] **Issue #163 — sign-extend narrow WDC4 signed-immediate fields.** The generic Rust WDC4
  reader previously returned an unextended `u32` payload from `get_field_i32` when a signed field
  occupied fewer than 32 bits. C++ explicitly extends `SignedImmediate` values before copying them
  into the requested signed type (`DB2FileLoader.cpp:858-869`). Rust now does the same while
  preserving raw unsigned access; synthetic bit-width fixtures and the real 3.4.3
  `SpellEffect.EffectBasePoints` data pin both paths. This fixes signed acquisition payloads and
  other existing `i32` consumers without treating the separate floating-point
  `world.serverside_spell_effect` source as regular DB2 metadata.
- [x] **Issue #164 — do not publish a newly inserted lower rank as active.** Legacy
  `Player::AddSpell` demotes a newly learned lower rank when a higher rank is already active, but
  returns the stale local `active` argument rather than the final `PlayerSpell::active` value
  (`Player.cpp:2855-2897,3135-3137`). `Player::LearnSpell` can consequently emit both
  `SMSG_SUPERCEDED_SPELL(low, high)` and a contradictory learned-spell publication
  (`Player.cpp:3192-3214`). The immutable Rust plan uses the final row state, retains the
  supersede intent, and deliberately omits the contradictory learned intent. A focused fixture
  distinguishes this bounded repair from ordinary higher-rank replacement.
- [x] **Issue #164 — reject skill-slot alias/capacity corruption instead of reproducing it.**
  Legacy `Player::SetSkill` uses `0` both as a valid array index and as “no free slot”, then
  activates parent/child skills after selecting but before claiming the slot
  (`Player.cpp:5799-5856`). Near capacity this can reject a genuinely free slot or let recursive
  activation reuse a stale position. Rust requires exact occupied-slot authority, activates
  causal parents/children, rechecks capacity, and returns a structured indeterminate outcome
  without exposing partial state. This is an intentional safety repair, not a claim that the
  legacy sentinel behavior was desirable protocol semantics.

---

## CRIT — data loss / duplication / corruption (fix before trusting the server with real chars)

- [x] **2026-10-01, live: deleting a character leaked every dependent row, including its
  items. Repaired the same day.** Reproduced over the wire with the server's own path
  (tools/wow-test-bot `--delete-characters`, C++ `CharDelete` →
  `Player::DeleteFromDB`). The four QA characters deleted successfully — the server answered
  `CHAR_DELETE_SUCCESS` and the `characters` rows are gone — and they left behind, measured
  immediately afterwards: **20 `character_inventory` rows, 20 `item_instance` rows,
  30 `character_skills`, 48 `character_glyphs`, 210 `character_reputation` and
  2 `character_homebind`** rows, for four level-1 characters that had never played.
  The cause is the adapter: `character_administration_adapter.rs:264` issues exactly one
  statement, `CharStatements::DEL_CHARACTER` (`DELETE FROM characters WHERE guid = ?`),
  while C++ `Player::DeleteFromDB`'s `CHAR_DELETE_REMOVE` branch issues **52** `CHAR_DEL_*`
  statements in one transaction (`Entities/Player/Player.cpp`, `Player::DeleteFromDB`).
  Severity is CRIT rather than cosmetic because the leaked `item_instance` rows keep their
  item GUIDs allocated for ever, the orphan rows accumulate on every delete, and a future
  character reusing a guid would inherit them: this is exactly the "trusting the server with
  real characters" bar. The orphan rows from this reproduction were removed by hand
  (`character_inventory`, `item_instance`, `character_skills`, `character_glyphs`,
  `character_reputation`, `character_homebind` for guids 1-4); nothing else on the QA account
  was touched. Owner: the A2 persistence lane and Part 2 **L24**.
  - **Repaired at `character_administration_adapter.rs`:** the delete now commits the full
    C++ `CHAR_DELETE_REMOVE` set — 49 CharacterDatabase statements in C++ append order —
    inside one transaction. No new SQL was needed: all 49 identities already existed and
    had no caller, and every one of their texts is byte-identical to the C++ statement it
    ports (checked statement by statement against
    `Database/Implementation/CharacterDatabase.cpp`).
  - **Scope contract, recorded on the function:** three C++ steps are deliberately not
    reproduced, because each needs a read or a second database this path does not have —
    the COD-mail refund and per-mail-id item deletes driven by `CHAR_SEL_CHAR_COD_ITEM_MAIL`
    and `CHAR_SEL_MAILITEMS`, the pet-id walk from `CHAR_SEL_CHAR_PET_IDS`, and the two
    `LOGIN_DEL_BATTLE_PET*` LoginDatabase statements. The unconditional mail and pet
    deletes still remove the character's own rows.
  - **Live proof:** a freshly created character (guid 6) held 156 rows across seven tables —
    1 `characters`, 5 `character_inventory`, 5 `item_instance`, 15 `character_skills`,
    24 `character_glyphs`, 105 `character_reputation` and 1 `character_homebind`. After
    `--delete-characters 6` every one of those counts reads **0**.
  - `the_delete_transaction_follows_the_cpp_append_order_and_binds_only_the_guid` pins all
    49 statements, their C++ order and their binds (only the guid, twice for
    `guild_eventlog`); `every_family_the_live_reproduction_leaked_is_now_deleted` pins the
    six families this reproduction measured.

- [x] **D-C1 Item enchantments not loaded on relog.** `SEL_CHAR_EQUIPMENT`/`SEL_CHAR_BAG_CONTENTS`
  select enchantment cols but the load hardcodes 0 → equipped/bagged enchants vanish on
  logout. `handlers/character.rs:4617-4618,4760-4761`. C++ `Player::_LoadInventory`.
  - 2026-07-01 issue #20 local slice: Rust now selects `item_instance.enchantments` for the
    specialized equipment/bag login queries, parses the 13x `(id,duration,charges)` fields like
    C++ `Item::LoadFromDB`, applies them to runtime `Item` objects, and includes them in item
    `CREATE_OBJECT` blocks. PR #89 merged with all required checks green. The issue-#20 closeout
    then loaded an enchanted/random-property item through both installed C++ and Rust runtimes,
    preserved the exact CharacterDB metadata around occupied forward/reverse swaps, and produced
    the same complete item-create block SHA-256 on both sides. Final reviewer hardening requires
    an observed empty-body logout packet in every phase, the second item's exact all-zero
    enchantment state, and a third Rust authentication proving the reverse-save reload:
    `25238a033be693b4969b9412f1666074e5d9be76c6db3b188e021a60b4feb2c8`.
- [x] **D-C2 Item random properties not loaded on relog.** Same query gap → magical items
  become non-magical. `handlers/character.rs:4617`.
  - 2026-07-01 issue #20 local slice: the same login path now loads `randomPropertiesId` and
    `randomPropertiesSeed` for equipped and bagged items into runtime item state and login create
    data. The same paired C++/Rust logout/relog proof above covers the nonzero property ID, seed,
    generated property enchantment and exact serialized item block.
- [x] **D-C3 Bank/equipment-set/void-storage persistence incomplete.** The original audit found
  these storage paths represented only in memory, with loss on logout.
  - 2026-07-13 issue #102 local slice: personal `AUTOBANK` / `AUTOSTORE_BANK_ITEM` now plans
    C++ `CanBankItem` / `CanStoreItem` destinations, commits every stack/location plus the
    surviving items' count/expiration/charges/flags/enchantments/durability/played-time and
    applicable quest-status change in one character transaction, and mutates runtime only after
    a successful commit. Fully absorbed sources also delete both stored-container-loot tables.
    Login now loads expiration/charges, normalizes template duration, limits charges to real
    ItemEffects, and restores duration trackers so that mutable-state save cannot overwrite them
    with defaults. Coverage includes empty destinations, merge+remainder, full bank,
    bank withdrawal, the C++ first-match stop and special item-push packet for quest-bound
    objectives (with no generic item-objective credit), equipment removal packet
    masks, merge-destination enchantment timer refresh without item-expiration registration,
    current enchantment durations, binding, obtain spells, a real failed-connection SQL commit
    regression, and fully merged metadata cleanup. The regression also repaired an old
    false-positive test that sent
    legacy slot `19` instead of the 3.4.3 backpack start slot `35`.
    PR #103 merged on 2026-07-14 after the full live bot bank deposit/relog/withdraw/relog
    round-trip passed. Personal-bank movement is therefore closed.
  - 2026-07-21 issue #112, merged as PR #113: equipment sets and transmog outfits now share one
    process-wide, startup-initialized GUID namespace like C++ `ObjectMgr`; initialization uses the
    exact combined CharacterDB maximum and fails closed. The existing player-save transaction
    persists new/changed/deleted rows, and login correctly decodes signed transmog schema values.
    A two-client installed runtime run proved concurrent distinct GUIDs, exact durable rows, two
    fresh-auth relogs with exact loaded sets, and cleanup. The committed one-packet
    `SMSG_EQUIPMENT_SET_ID` action capture is byte-clean against C++ on the instance route with no
    accepted divergence. Equipment-set persistence is therefore closed.
  - 2026-07-22 issue #114 / PR #115: void storage now loads into a validated fixed 160-slot
    authority and uses one process-wide, startup-initialized item-ID generator like C++
    `ObjectMgr`. Unlock, query, transfer and swap enforce the represented C++ gates. Deposit,
    withdrawal and swap commit flags/money, inventory/item mutations and all affected void rows
    in one CharacterDB transaction before publishing runtime or success packets; definite
    rollback stays invisible and indeterminate COMMIT is fenced from stale saves. This explicitly
    accepts issue #114's failure-only divergence from C++ intermediate deposit publication when a
    later withdrawal fails validation; the issue's Done contract requires one transaction and no
    runtime change on definite failure. C++ packet
    contrast also corrected every void-storage GUID from a fixed 16-byte Rust/bot encoding to
    C++ `PackedGuid`. Focused tests, an installed unlock/deposit/relog/swap/relog/withdraw/relog
    lifecycle with exact cleanup, and a 1/1 byte-clean real C++/Rust query capture have passed.
    Review hardening also restores C++ random-property/suffix enchantment slots on withdrawal and
    persists the same effective enchantment array, so a later save/relog cannot strip item affixes.
    Locked-character login now skips residual void rows like C++ while initializing coherent empty
    storage; unlock deletes those skipped rows in the same money/flag transaction, so neither a
    same-session query nor a restart can expose contents C++ never loaded.
    Withdrawal now honors C++ merge-before-empty `CanStoreNewItem` placement across the entire
    atomic request while excluding stacks/children already planned for deposit destruction, login
    replays each valid row's represented collection appearance hook, and swap destination values
    preserve C++'s `uint32` to `uint8` truncation before range checks.
    GitHub review also published a withdrawn item's pre-random/pre-handler `CREATE_OBJECT` and its
    post-store random-property/creator/binding VALUES update before the slot update, and capped
    allocation at the packet GUID's 40-bit counter so raw IDs cannot alias after truncation.
    Context-column, void-packet random-affix, and fixed-scaling review suggestions are intentionally
    not applied because exact C++ contrast confirms the existing Rust behavior in all three cases.
    A later current-HEAD review also adds an explicit older-Rust compatibility repair that restores
    the C++ schema default for legacy zero-slot characters, atomically plans and persists
    item-objective quest state across ordered deposit destruction (including bag children) and
    withdrawal credit, preserves intermediate recursive removal checks plus quest-bound
    no-physical-item withdrawals, and sends live collection updates for both new and merged
    physical withdrawals. These latest fixes pass focused void/capacity/quest/collection tests;
    the complete local PR preflight and local Codex review completed CLEAN on `2143334b` in 471.8
    seconds. PR #115 merged as `55719eb4` with CI and the current-HEAD Codex verdict green. Bank,
    equipment/transmog sets and void storage are therefore all closed for the scoped D-C3 paths.
- [x] **D-C4 Inventory swap not transactional.** Two separate `execute()` calls; mid-fail
  orphans/dupes items. `handlers/character.rs:11668-11681`. C++ appends both changed positions to
  the character save transaction through `Player::_SaveInventory`.
  - 2026-07-14 issue #104 local slice: every represented direct-inventory `SwapItem` route now
    appends the final positions to one character transaction with C++
    `CHAR_REP_INVENTORY_ITEM`/`REPLACE INTO` semantics. `REPLACE` is required for occupied swaps
    because `character_inventory.uk_location` forbids the first half of a two-`UPDATE` exchange.
    Runtime slots, equipment modifiers, accessor/registry state, loot release, stat/value packets,
    and success logging now occur only after commit; missing/failed persistence sends
    `SMSG_INVENTORY_CHANGE_FAILURE` and leaves runtime unchanged. Focused coverage includes empty
    and occupied plans, generic commit failure, explicit auto-equip-slot failure, and all 2,763
    `wow-world` library tests. The 2026-07-14 release-build live QA passed both the isolated bot
    round-trip (occupied swap, logout, full re-auth/relogin, inverse swap, second persistence
    check) and a manual client swap/relogin check. An accidental debug-binary deployment was
    rejected after a stack overflow and replaced with the verified release artifact before these
    passes. PR #105 merged with every required check and the current-HEAD Codex verdict green.
    The issue-#20 closeout reran both occupied swaps through logout/fresh-auth and exact DB checks.
  - Separate validation follow-up, 2026-07-22 issue #52 local slice: the live move/equip/store
    handlers now use C++ `IsValidPos`, bank-interaction, `CanUnequipItem`, `CanStoreItem`/
    `CanBankItem`, `CanEquipItem`, bag, unique-equip and recursive-destroy rules instead of the
    former direct-inventory simplification. `Player::SwapItem` now covers empty moves, merges,
    bidirectional real swaps, bag exchanges, child redirects and persisted offhand follow-up, with
    each concrete mutation committed before runtime publication. Paired installed C++/Rust QA
    proved the invalid container-aware source error on the C++ realm route plus forward/reverse
    occupied swaps and fresh-auth metadata; strict capture-diff matched request and response with
    zero value/routing/count differences. This closes the bounded #LegacyAudit.ITEM.2 behavior, not broader
    item/gem/durability parity, and remains pending PR CI/current-HEAD review/merge. GitHub review
    additionally applied current upstream TrinityCore's missing legacy `AutoUnequipChildItem`
    pre-step before child redirects and stopped internal inventory relocations from re-crediting
    quest objectives. Proposed `CanUseBank` guards for auto-equip/auto-store were not applied:
    both the local 3.4.3 source and current upstream omit them, while the swap handlers retain them.
- [x] **D-C5 Loot item TOCTOU → duplication.** Slot marked looted *after* the async inventory
  store; two concurrent looters both store it. `handlers/loot.rs`. C++ instead gets safety from
  object-owned `Loot` plus globally serialized `PROCESS_THREADUNSAFE` session work; it validates
  storage before mutating the shared slot.
  - 2026-07-18 issue #106 local slice: creatures/gameobjects now own a generation-tagged shared
    loot authority; item/master/roll/disenchant paths use cancellation-safe leases whose detached
    persistence worker owns the claim across SQL `COMMIT`. Session loot tables are packet caches,
    and stale corpse/GO generations and stale group rolls fail closed. The guarded two-bot race,
    single-session C++/Rust capture, original-client QA, CI and current-HEAD review all completed;
    PR #107 merged.
- [x] **D-C6 Loot money TOCTOU → duplication.** `loot.coins` zeroed *after* distribute; two
  concurrent `handle_loot_money` both pay out. `handlers/loot.rs`.
  - 2026-07-18 issue #106 local slice: one detached worker atomically persists every connected,
    allowed, in-range group share, commits the object-owned money claim, and then schedules one
    exact-once runtime application per session without cross-session acknowledgement waits. A
    cancelled packet future cannot reopen a successful DB transaction. The same required gates
    completed in merged PR #107.
  - **Separate crash-recovery boundary (does not reopen D-C5/D-C6):** these detached workers and their
    completion trackers are in-process only. A runtime/process abort exactly after SQL `COMMIT`
    but before the synchronous authority/completion continuation can still lose that continuation.
    Closing `kill -9` recovery requires a durable claim journal written in the same transaction
    and replayed at startup; neither the current authority nor the session tracker is that journal.
- [x] **D-C7 Player save had incomplete transaction coverage.** Issue #17 / PR #88 adds the
  periodic save timer and wraps the Rust-covered represented `Player::SaveToDB` statements in one
  `SqlTransaction`, clearing dirty state only after commit. Automated runtime QA covers
  login/logout, inactive action rows, travel columns and unchanged quest-objective rows; manual
  original-client QA confirmed logout/relog plus action-bar and cooldown persistence. PR #88
  merged with CI and Codex review green. The issue-#20 paired C++/Rust run additionally verifies
  the observable logout envelope, including C++'s realm-routed empty
  `SMSG_LOGOUT_COMPLETE`. Full C++ save breadth and login/account cross-database coupling remain
  Part-2 parity work, not an open instance of this scoped CRIT transaction defect. `session.rs`.
- [x] **D-C8 Vendor buy not atomic.** Gold/currency applied to runtime before item DB commit;
  commit fail = paid, no item. `handlers/character.rs:10177-10292`.
  - 2026-07-20 issue #108 local slice: ordinary item purchases already gained a combined
    gold/item/turn-in transaction in #107, but item extended-cost currencies and the entire
    currency-vendor branch still changed session currency before awaiting COMMIT. Both paths now
    build detached currency plans and publish them only after the purchase transaction commits.
    Currency-only purchases reuse the cancellation/unknown-COMMIT quarantine with equal money
    markers, so definite rollback leaves runtime untouched and an ambiguous result requires relog
    without allowing a stale full save. A failed-connection handler regression exercises the real
    rollback branch and proves that it emits only `BuyFailed`, preserves runtime currency, and
    reopens payout/save admission. Paired C++/Rust bot QA now proves a real extended-cost purchase,
    currency debit, item creation, fresh-authentication persistence, packet routing, and cleanup;
    the committed post-COMMIT realm response is 2/2 CLEAN with no accepted divergences. Capture
    contrast also fixed zero-price Coinage publication and C++ vendor-item create/context/flag
    metadata. The wider action still shows the separately scoped missing achievement
    `SMSG_CRITERIA_UPDATE`. Installed original-client QA on 2026-07-21 bought two extended-cost
    items across a relog and confirmed exact item/currency persistence in CharacterDB; the fixture
    was then fully restored. The confusing client `You receive currency` line was backed by the
    same byte-exact loss packet as C++ (quantity 15, delta -15, Vendor reason), not a refund.
    PR #109 merged after final CI and the current-HEAD Codex verdict passed.
- [x] **D-C9 Group full-check race.** Size checked then join without re-check → 6+ member
  groups under concurrent accepts. `handlers/group.rs:928-1044`.
  - 2026-07-21 issue #110 local slice: C++ checks `Group::IsFull` immediately before
    `Group::AddMember` on its serialized execution path. Rust now performs that pair under one
    mutable `GroupRegistry` guard and returns explicit Full versus AddFailed results to the live
    handler; `ERR_GROUP_FULL` leaves the rejected session and group unchanged, while AddFailed
    retains C++'s silent return. A barrier-synchronized
    regression starts two simultaneous joins for the fifth party slot, proves exactly one Added
    plus one Full result, and verifies the final member count remains five. The installed
    `4adf87e1` runtime and three-client bot race passed locally on 2026-07-21: one candidate
    received exact `Invite/GROUP_FULL`, the other joined, CharacterDB contained exactly the four
    initial members plus that winner, all sessions logged out, and the fixture was restored.
    PR #111 merged after final CI and the current-HEAD Codex verdict passed.

## HIGH — broken mechanics / silent failure / exploit

- [x] **D-H1 and D-H2 are stale notes, closed on contrast 2026-10-01, not by new work.** Both
  pointed at `session.rs:79xx`/`:478xx`, a file that no longer exists, and both were overtaken by
  the #29/#61 slices. The current swing in
  `session/mod.rs::represented_white_swing_damage_like_cpp` runs, in C++'s order:
  `MeleeDamageBonusDone`'s flat and percentage pair, the autoattack multiplier,
  `MeleeDamageBonusTaken`, `CalcArmorReducedDamage` with armour penetration, the target's
  resistance aura and the caster bypass, then the attack-table roll, then the outcome switch with
  its own damage. `session_rules::melee_outcome_damage_like_cpp` carries every C++ arm — immune,
  evade, miss, dodge, parry, glancing with the level-difference reduction, block with
  `GetBlockPercent`, crit with the damage multiplier, and the crushing arm kept with its C++
  source expression — and `melee_outcome_presentation_like_cpp` publishes the real
  `HitInfo`/`VictimState` instead of a hardcoded pair. Both entries are closed as **inaccurate
  records**; nothing was implemented for them here.
- [x] **D-H3 Spell hits had no coefficient, no critical and no resist. All four stages are now
  done.** Restated on contrast 2026-10-01, because the original one-liner was wrong in two
  directions; closed 2026-10-02 when the absorb stage landed.

  * **Coefficient: already done before this session.** `SpellDamageBonusDone` is ported with
    `BonusCoefficient`, `BonusCoefficientFromAP`, the `SPELL_ATTR3_IGNORE_CASTER_MODIFIERS`
    short circuit and the percentage chain
    (`session/spell_effects/effect_combat.rs:614-668`), and the healing side with it.
  * **Critical: done 2026-10-01**, see the entry below.
  * **Resist: done 2026-10-01**, see D-H21 below.
  * **Absorb: done 2026-10-02.** `represented_spell_absorb_for_damage_like_cpp`
    (`session/spell_effects/spell_absorb.rs`) runs C++'s school-absorb loop
    (`Unit.cpp:2114-2178`) between the resist and `DealDamage`: it spends each shield in
    `AbsorbAuraOrderPred` order through the canonical aura amount, publishes one
    `SMSG_SPELL_ABSORB_LOG` per consuming shield before the damage log, removes a spent shield with
    its slot update, and `SMSG_SPELL_NON_MELEE_DAMAGE_LOG` carries the real `absorb`.

    Two sentences of the paragraph this replaces were wrong, and saying so is part of the record.
    The mutable amounts were **not** player-only: `creature_absorb_shields_like_cpp` had been reading
    a creature's `applied_aura_amounts` since the melee creature-victim work. And the player victim
    was not the easy side — it has no spell-damage path at all, because
    `apply_damage_from_caster_like_cpp` resolves a creature target or returns and the creature spell
    tick executes no effects. The implemented victim is therefore the creature, and the player victim
    waits on creature spell effect execution, not on aura ownership.

    Boundaries kept: `SPELL_AURA_MANA_SHIELD` has no creature-side projection, so C++'s second loop
    (`:2179-2248`) is empty for this victim, exactly as it is for the melee creature victim; the
    absorb scripts and their `defaultPrevented` escape (`:2140-2144`) are not ported, so an
    infinite-absorb shield stays clamped to zero; the spell block stage is still zero.

    Evidence: four scenarios in `session/tests/scenarios_spell_state_27.rs` — the shield spent with
    its remainder surviving, a spent shield removed with its aura update and the rest of the hit
    landing, the resist running first so the shield only sees what it left, and a shield of another
    school absorbing nothing — plus `cargo test -p wow-world --lib`, 4278 passed. **Not live**: no
    client-reachable shape exists yet in either direction. A player cannot shield a hostile creature
    (absorb spells are self or friendly target, and this server has no GM command surface) and a
    creature cannot cast damage at a shielded player (the tick above). The live run is owed when
    creature spell effects land, and is not claimed here.
- [x] **D-H20 Spell hits could never crit, in either direction.** Implemented 2026-10-01 as the
  critical half of D-H3. The damage path applied no critical at all — the code said so in a
  comment — so a caster's spell crit percentage, which the port already computes per school on
  the Player, changed nothing, and `SMSG_SPELL_NON_MELEE_DAMAGE_LOG` never carried
  `SPELL_HIT_TYPE_CRIT`. Heals were the same: the log's `Crit` was a hardcoded `false`.

  Ported as the chain C++ runs: `SPELL_ATTR0_CU_CAN_CRIT` as a store predicate, because this
  port has no `AttributesCu` — the eleven effects at `Spells/SpellMgr.cpp:3367-3381` minus
  `SPELL_ATTR2_CANT_CRIT` at `:3643-3645`; `Unit::SpellCritChanceDone`
  (`Entities/Unit/Unit.cpp:7706-7772`) with its three early returns and the damage-class switch
  that takes the larger of the physical and magical branches; `Unit::SpellCritChanceTaken`
  (`:7774-7960`), including the arm that makes `SPELL_DAMAGE_CLASS_NONE` return zero however
  large the done chance was; the single `roll_chance_f` C++ makes per target
  (`Spells/Spell.cpp:8675-8684`); and the two different damage arms —
  `Unit::SpellCriticalDamageBonus` for magic (`:7962-8003`) and the doubling arm
  `CalculateSpellDamageTaken` runs inline for weapon-based spells (`:1266-1298`) — plus
  `Unit::SpellCriticalHealingBonus` (`:8005-8036`), which is not the damage function renamed: it
  computes the bonus alone, multiplies *that* by `MOD_CRIT_PERCENT_VERSUS`, adds it only when
  positive, then multiplies the whole heal by `MOD_CRITICAL_HEALING_AMOUNT`.

  **Named boundaries**, each a fact the represented runtime does not carry: every crit-chance and
  crit-damage aura term is zero, and so is resilience, because those aura families are not
  represented; `SpellInfo::IsPositive` is not represented, so the damage path passes harmful and
  the heal path positive, which is what each one is by construction; a creature victim is taken
  as standing, which is the stand state a represented creature has, so the always-crit-sitting
  rule is reachable only for a player victim this path does not serve; the `SpellModOp::CritChance`
  and `CritDamageAndHealing` spellmods need the talent spellmod owner; and the scripted class
  blocks in `SpellCritChanceTaken` (`:7799-7930`: Shatter, Glyph of Shadowburn, Renewed Hope and
  the per-family cases) need `SPELL_AURA_OVERRIDE_CLASS_SCRIPTS` effects this port has no
  representation for.

  **Excluded deliberately:** the reference fork's `alistar:`-marked warlock healthstone branch in
  `getPhysicalCritChance` (`:7729-7736`). A patched region is not parity evidence for this build,
  so the unpatched shape is what was ported, and the exclusion is recorded at the rule.

  **Proven live on 2026-10-01, from a twenty-cast sampling run.** The run published two damage logs
  and the second is the critical:

  * `cast 1: damage=10 original=13 resisted=3 absorbed=0 school=0x04 flags=0x00`
  * `cast 2: damage=14 original=19 resisted=5 absorbed=0 school=0x04 flags=0x02`

  Every number in the second row is the C++ arithmetic. `flags = 0x02` is `SPELL_HIT_TYPE_CRIT` on
  the wire. `original_damage` is **19** where an ordinary hit of the same spell is 13, which is
  `SpellCriticalDamageBonus`'s magical arm exactly — `13 + 13/2` truncated to 19
  (`Entities/Unit/Unit.cpp:7962-8003`). And the resist composed with it in C++'s order, not before
  it: the critical raised the damage, `originalDamage` was assigned from that, and the resist then
  took 5 of the 19, leaving the 14 the creature received. That ordering is
  `CalculateSpellDamageTaken` followed by `CalcAbsorbResist` (`:1319-1347`), so the run is evidence
  for D-H21's composition as much as for this entry's roll.

  Reproduce with `--spell-damage 133 --spell-damage-entry 475 --spell-damage-character 6
  --spell-damage-casts 20` on a freshly started server. The mode fills the caster's mana first,
  because a drained caster is refused with `SPELL_FAILED_NO_POWER` — correctly, see the withdrawn
  D-H23. The deterministic scenarios keep both pinned outcomes beside this.
- [x] **D-H4 Quest kill-credit — verified working on a live kill, 2026-10-01.** The contested
  reading is settled in favour of "monster kills advance". Quest 14106 was seeded as
  incomplete for the QA character (a fixture: the bot cannot take a quest from an NPC yet),
  its MONSTER objective names entry 721, and killing a Rabbit published
  `SMSG_QUEST_UPDATE_ADD_CREDIT` and moved `character_queststatus_objectives.data` from 0 to
  1, persisted. Reproduce with `--loot-after-kill --melee-creature-entry 721`.

  **One real gap was found and repaired while verifying it.** C++ `Player::KilledMonster`
  (`Entities/Player/Player.cpp:16561-16571`) credits the creature's own entry **and** each
  non-zero `CreatureTemplate::KillCredit`, each through `KilledMonsterCredit` with an empty
  guid. RustyCore credited only the entry, so a "kill X" objective naming a credit proxy —
  the usual shape when several creatures count for one objective — could never advance;
  `creature_template.KillCredit1/2` was loaded for the client's creature query and read by
  nothing else. The proxies are now expanded on the kill-reward path, which is C++'s single
  `KilledMonster` caller (`Entities/Player/KillRewarder.cpp:181`), and the kill-credit spell
  effect was corrected to the single-entry `KilledMonsterCredit` C++ uses there
  (`Spells/SpellEffects.cpp:5437`).

  **Named boundary:** a kill by spell damage passes no credit proxies, because the composition
  root deliberately keeps the ObjectMgr query catalogs out of `WorldSession` — pinned by
  `session_resources_requires_named_capability_bundles` — and that path has no catalog in
  scope. Written on the call site rather than left silent.

- [x] **2026-10-01, live: a critter meleed the player back — repaired.** A Rabbit (entry 721,
  `creature_template.type = 8` `CREATURE_TYPE_CRITTER`) published 47
  `SMSG_ATTACKER_STATE_UPDATE` against the player over 120 seconds after being attacked.

  The mechanism, read rather than assumed: `ThreatManager::CanHaveThreatList`
  (`Combat/ThreatManager.cpp:172-190`) does **not** exclude critters, and
  `Creature::Update` reaches `DoMeleeAttackIfReady()` centrally for every creature
  (`Entities/Creature/Creature.cpp:921-932` — a region the fork patches, but only to collapse
  a pet branch into the same unconditional call), so neither the threat list nor the AI's
  `UpdateAI` is what stops the swing. The one thing that does is
  `Unit::DoMeleeAttackIfReady`'s early return on `!Creature::CanMelee()`
  (`Entities/Unit/Unit.cpp:2433-2434`), and the flag behind it is written by the AI
  constructors that call `SetCanMelee(false)`: `TurretAI` (`AI/CoreAI/CombatAI.cpp:200`),
  `VehicleAI` (`:234`), `PassiveAI` (`AI/CoreAI/PassiveAI.cpp:25`) and `NullCreatureAI`
  (`:36`), with `CritterAI` deriving from `PassiveAI` and `TriggerAI`/`TotemAI` from
  `NullCreatureAI`. `PossessedAI` sets only `REACT_PASSIVE` and is excluded.

  RustyCore already enforced one of those at the global melee boundary, but by comparing the
  database `AIName` against the string `"TurretAI"`. A critter's row leaves `AIName` empty and
  receives `CritterAI` from the Permissible scoring (`AI/CoreAI/PassiveAI.cpp:95-100`), so it
  was invisible to a string match. The gate now asks the resolved AI kind through
  `creature_ai_sets_no_melee_like_cpp`, which covers every one of those constructors and
  subsumes the old string test.

  Live, before and after: `creature_landed=47` became `creature_landed=0`, with one player
  swing killing the rabbit. Regression on an ordinary hostile creature in the same session:
  entry 94 still retaliates (`creature_landed=3`), dies, pays 44 XP and drops 9 copper plus
  two items. Reproduce with `--loot-after-kill --melee-creature-entry 721` and `… 94`.

  **Not ported here:** `Creature::InitializeReactState` (`Creature.cpp:1357-1367`), which also
  makes totems, triggers, critters and spirit services `REACT_PASSIVE`. RustyCore already
  suppresses their `MoveInLineOfSight` aggro through the same AI-kind selection, so the
  observable aggro behaviour matches; the react-state field itself is still unset for them.
- [x] **D-H5 Quest area-trigger (explore) objectives were not wired, in three separate
  places.** Closed 2026-10-01. The old note named one of them; reading the whole operation
  found a data path that was never composed, a handler block that did not exist, and a
  completion rule that could not recognise the result.

  1. **The relation store was never loaded.** `QuestAreaTriggerStoreLikeCpp` and its
     faithful loader (C++ `ObjectMgr::LoadQuestAreaTriggers`,
     `Globals/ObjectMgr.cpp:6470-6532`) already existed in `wow-data`, as did the SQL and
     the persistence port, but nothing composed them: the comment in
     `world-server/src/area/trigger_world_catalog.rs` said the quest-relation operation
     "remains dormant until its owners compose them". Composed now after the quest store,
     which is the C++ order, because every row is validated against it.
  2. **`HandleAreaTriggerOpcode` had no quest block.** C++ credits area-trigger quests
     there (`Handlers/MiscHandler.cpp:530-574`), before the tavern branch and only for a
     living player entering the trigger. RustyCore went from the script dispatch straight
     to the tavern handling, which returns, so even a trigger that was both would have lost
     its quest. The port keeps C++'s own reason for not using
     `Player::UpdateQuestObjectiveProgress` (its comment at `:532`): a
     `quest_objectives.ObjectID` of `-1` means "any trigger bound by
     `areatrigger_involvedrelation`", so the quests come from the relation store and the
     objective's id is only a filter.
  3. **No flag-storing objective could ever be complete.**
     `represented_quest_objective_complete_like_cpp` knew the counter types and the progress
     bar and returned `false` for everything else, so `QUEST_OBJECTIVE_AREATRIGGER` stayed
     incomplete no matter what was stored. C++ `Player::IsQuestObjectiveComplete`
     (`Entities/Player/Player.cpp:16982-16990`) groups the flag-storing types apart and
     completes them on any non-zero stored value. That group is now ported — types 10, 11,
     12, 14, 19 and 20 — which also moved `QUEST_OBJECTIVE_CRITERIA_TREE` out of the
     counter group, where it did not belong.

  Evidence: two scenario tests drive the real handler from a `CMSG_AREA_TRIGGER` packet to
  the credited objective and the `SMSG_QUEST_UPDATE_ADD_CREDIT_SIMPLE` on the wire, and
  cover the `ObjectID = -1` case and the refusal of an objective naming a different
  trigger. Live, the store now loads with real data where it previously loaded nothing:
  `Loaded 82 C++ quest area triggers (57 rows seen, 49 from relations, 35 from objectives;
  0 skipped missing AreaTrigger.db2, 0 skipped missing quest, 8 skipped obsolete quest)`.

  **Proven live the same day.** tools/wow-test-bot gained an `--area-trigger` mode and the
  server an operator trace for the geometry `AreaTrigger.db2` holds and SQL does not, so the
  position comes from the server rather than from a guess: trigger 87 is map 0,
  `(-9077.34, -552.92, 60.35)`, radius 30. Standing there with quest 76 "The Jasperlode
  Mine" incomplete and sending `CMSG_AREA_TRIGGER` published
  `SMSG_QUEST_UPDATE_ADD_CREDIT_SIMPLE` (9 bytes: quest, object, type) and one
  `SMSG_QUEST_UPDATE_COMPLETE`, and after the clean logout
  `character_queststatus_objectives.data = 1` with `character_queststatus.status = 1`
  (complete) and `explored = 1`.

  The mode needed one thing the scenario tests never show: C++ gates the block on
  `player->IsAlive()` (`MiscHandler.cpp:530`), and a trigger position is where the quest's
  mobs are, so a level-2 character parked at the mine dies there. One run produced zero
  credits for exactly that reason and the server was right to refuse it, so the mode now
  restores a character a previous run left dead and reports `revived=true`.

  The live run also caught a wire divergence the tests could not: the first run sent **two**
  `SMSG_QUEST_UPDATE_COMPLETE`, one from the objective path and one from the explore path.
  C++ sends one. `Player::CompleteQuest` (`Entities/Player/Player.cpp:14947-14971`) sets the
  status and the quest-log slot state and publishes nothing at all; the only packet is
  `AreaExploredOrEventHappens`'s own `SendQuestComplete` (`:16507`), which fires once, when
  `Explored` flips. The objective path no longer sends its copy.

  **Also left open:** `IsQuestObjectiveComplete`'s live-state branches.
  `QUEST_OBJECTIVE_MIN_REPUTATION` / `MAX_REPUTATION` ask `GetReputationMgr`, `MONEY` asks
  `HasEnoughMoney`, `LEARNSPELL` asks `HasSpell` and `CURRENCY` asks `HasCurrency`
  (`Player.cpp:16970-16998`). The Rust rule is pure — status and quest only — so each needs
  that state threaded in. They still fail closed, which leaves a quest incomplete rather
  than completing it on an unchecked condition. Recorded as D-H17.
- [x] **D-H17 Five objective types could not complete, because the completion rule carried no
  live player state.** Closed 2026-10-01. C++ `Player::IsQuestObjectiveComplete`
  (`Entities/Player/Player.cpp:16970-16998`) decides five of its branches by asking the
  Player directly rather than reading stored progress: `MIN_REPUTATION` (6) and
  `MAX_REPUTATION` (7) ask `GetReputationMgr().GetReputation(ObjectID)`, `MONEY` (8) asks
  `HasEnoughMoney(Amount)`, `LEARNSPELL` (5) asks `HasSpell(ObjectID)` and `CURRENCY` (4)
  asks `HasCurrency(ObjectID, Amount)`. `represented_quest_objective_complete_like_cpp` is
  pure — status and quest only — so all five fell to its `_ => false`, and any quest whose
  completion depended on one of them could never be finished.

  In the installed world database that is **256 quests**: 165 objectives on
  `MIN_REPUTATION`, 90 on `MONEY` and one on `MAX_REPUTATION`. `CURRENCY` and `LEARNSPELL`
  have no rows in 3.4.3 data, and are ported anyway because C++ has them.

  The rule stays pure. `RepresentedQuestObjectivePlayerFactsLikeCpp` is one borrowed
  snapshot of exactly what C++ asks the Player for, resolved by the owner before the rule
  runs and only for the ids that quest's own objectives name — one or two entries, not the
  whole reputation list. A quest with none of those five types resolves to the default and
  costs no session read at all. No trait per helper, no universal context, no second mirror:
  the facts are a value, and the four readers behind them
  (`resolved_player_money_like_cpp`, `with_reputation_mgr_like_cpp`,
  `known_spells_like_cpp`, `player_currencies_like_cpp`) already had owners.
  `stored_progress_only_player_facts_like_cpp` names the case where a caller provably cannot
  need them, which is how `Player::HasQuestForGO`'s gameobject scan avoids paying for one;
  that pairing moved into the rules as
  `represented_gameobject_objective_is_pending_like_cpp`, which took three lines *out* of
  `session/mod.rs`.

  Each of the five branches has a positive and a negative test, including the two boundaries
  C++ states precisely: `HasEnoughMoney(int64)` treats a negative requirement as satisfied
  (`Entities/Player/Player.h:1663-1664`), and `HasCurrency` needs the currency to be present
  *and* at least the amount (`Entities/Player/Player.cpp:7250-7254`).

  Live: quest 13265 "Cloth Scavenging", whose only objective is `MONEY` for 50000, seeded
  incomplete with the character holding 49995. Looting 11 copper from one kill took it to
  50006 and the quest's persisted status went from `3` (incomplete) to `1` (complete). The
  path is the real one — the loot-money change enqueues
  `RepresentedQuestObjectiveProgressEventLikeCpp::MoneyChanged`, whose drain asks
  `represented_can_complete_quest_after_objective_like_cpp` — and before this repair that
  call could not return true for a money objective.

  **Noticed in passing, not repaired here:** `reputation_for_faction_like_cpp` returns
  `base_reputation + standing.unwrap_or(0)`, where C++ `ReputationMgr::GetReputation`
  returns `0` outright for a faction the player has no `FactionState` for
  (`Reputation/ReputationMgr.cpp:183-193`). For a faction whose race/class base is non-zero
  the two disagree. It is pre-existing and shared with every other reputation consumer, so
  it belongs to its own change rather than to this one; recorded as D-M18.
- [x] **D-M18 withdrawn: it was a misreading, and the Rust reputation reader is faithful.**
  Raised and withdrawn 2026-10-01, the same day. The claim was that
  `reputation_for_faction_like_cpp` over-reports because it returns
  `base_reputation + standing.unwrap_or(0)` where C++
  `ReputationMgr::GetReputation(FactionEntry const*)` returns `0` outright for a faction with
  no `FactionState` (`Reputation/ReputationMgr.cpp:183-193`).

  That `return 0` is unreachable for any faction the comparison is about.
  `ReputationMgr::Initialize` (`:395-426`) inserts a `FactionState` with `Standing = 0` for
  **every** faction whose `CanHaveReputation()` holds, and `CanHaveReputation()` is exactly
  `ReputationIndex >= 0` (`DataStores/DB2Structure.h:1280-1283`) — the same predicate as
  Rust's `can_have_reputation_like_cpp`. So for a faction the player has never touched, C++
  computes `GetBaseReputation + 0`, which is precisely what Rust's `unwrap_or(0)` computes.
  The Rust port also calls its own `initialize_like_cpp` in production
  (`session/progression/reputation.rs:243`), with its own regressions for the
  reputation-faction-only state list, the race/class slot choice, friendship factions and
  paragon flags.

  Recorded rather than deleted: the finding was published in a commit message and a PR
  before it was checked, and the correction belongs next to it.
- [x] **D-H18 Player auras were loaded but never saved, so every buff and debuff died at
  logout.** Opened and implemented 2026-10-01; live evidence below.

  `Player::_LoadAuras` is composed in production — `handlers/character/world_entry.rs:2447`
  reads `character_aura` and `character_aura_effect` and installs the applications — but there
  was no write side anywhere. The only `save_auras` in the tree was the pet's
  (`pet/ops_2.rs:207`), `PlayerCharacterSaveRequestLikeCpp` had no aura group, and the
  `CharStatements::{DEL_CHAR_AURA, DEL_CHAR_AURA_EFFECT, INS_AURA, INS_AURA_EFFECT}`
  statements existed with no caller. A character therefore logged back in with exactly the
  auras of its last successful *write*, which was never, so with none.

  Ported as C++ builds it, in the layers that own each part:

  * `Aura::CanBeSaved` (`Spells/Auras/SpellAuras.cpp:1172-1209`) and `Aura::GenerateKey`
    (`:1262-1281`) as pure rules in `wow-entities/src/unit_subsystems/aura_save.rs`, with the
    two masks **derived** from the live effect list the way `GenerateKey` derives them, so a
    caller cannot hand in a mask that disagrees with the effect rows beside it.
  * `Player::_SaveAuras`'s statement order (`Entities/Player/Player.cpp:20089-20146`) as the
    plan: `DEL_CHAR_AURA_EFFECT`, `DEL_CHAR_AURA`, then each kept aura followed by its own
    effect rows. Both deletes are appended before C++ reads `m_ownedAuras`, so an empty aura
    list still clears the stored rows; a group that could not be read is `None` and touches
    neither table.
  * the group's place in the transaction: between the action buttons and the equipment sets,
    which is C++'s `_SaveActions` → `_SaveAuras` (`Player.cpp:19947-19948`). The frozen
    statement-order fixture gained exactly those four entries at that point and nothing moved.
  * the three `SpellInfo` predicates `CanBeSaved` consults, none of which existed:
    `IsSingleTarget` (`SpellInfo.cpp:1789-1796`, `SPELL_ATTR5_LIMIT_N` alone), the
    area-effect loop (`IsTargetingArea` + `IsAreaAuraEffect`, `SpellInfo.cpp:452-489`,
    including the complete `AREA`/`CONE` set from `SpellImplicitTargetInfo::_data`), and
    `SPELL_ATTR0_CU_AURA_CANNOT_BE_SAVED`. The last has no `AttributesCu` field here, so its
    rules are read off the stores at the point of use: the aura-type list at
    `SpellMgr.cpp:3340-3362` and the `LeaveWorld` interrupt flag at `:3604-3605`.

  **Named boundaries, written at the call site rather than left silent.** Each is a fact the
  represented runtime does not carry, not a choice:

  * `castItemId` / `castItemLevel` are written as zero, because the represented aura carries
    neither. The load side ignores both columns too, so the round trip is self-consistent.
  * `remainCharges` is written as zero, because charge consumption is not tracked for Player
    auras. `_LoadAuras` restores the spell's full `ProcCharges` for a stored zero, which is
    the same branch it takes for a C++ aura that never spent a charge.
  * per-effect `baseAmount` is the effect's `BasePoints`, which is what
    `AuraEffect::AuraEffect` (`SpellAuraEffects.cpp:620`) computes when no stored base amount
    was loaded. The port's `_LoadAuras` does not retain loaded base amounts, so a saved aura
    comes back with the data value rather than its own.
  * `recalculateMask` is the full effect mask, because `AuraEffect::m_canBeRecalculated`
    starts true (`:622`) and is only cleared by a script amount handler this port does not run.
  * the third source of `SPELL_ATTR0_CU_AURA_CANNOT_BE_SAVED`, liquid auras
    (`SpellMgr.cpp:3649-3655`), reads `LiquidType.db2::SpellID`, which no store here loads.
  * the installed `character_aura` has thirteen columns; the reference fork also writes
    `critChance` and `applyResilience`. The port writes the thirteen that exist.

  A permanent aura round-trips through the C++ `-1` marker: this port represents a permanent
  aura as `duration_total == 0` (`spell_state/aura_application.rs:880-882`) and writes `-1` for
  both duration columns, which is the value the loader's own permanent branch reads back
  (`session/mod.rs:1586-1602`).

  **Live evidence, 2026-10-01** (`tools/wow-test-bot --aura-save 6673`, exit 0, reproduced
  twice). Seeding and then finding the row still present proves nothing — that is also what a
  missing save looks like — so the fixture seeds two rows and the check is the difference
  between them. A row for spell `90000001`, which no `Spell.db2` carries and `_LoadAuras`
  therefore drops, **did not survive** the logout: the table really was cleared and rewritten.
  The Battle Shout row survived with every seeded value replaced by the live one:
  `recalculateMask` 0 → 1, `remainCharges` 5 → 0, and the effect's `baseAmount` 777777 → **14**,
  which is that effect's `BasePoints`. `casterGuid` came back as 16 binary bytes. The relog
  published `SMSG_AURA_UPDATE` twice, so what the save wrote came back as a live aura.

  **A defect this introduced, caught by the tests before it reached the server.** The first
  wiring asked the session for the Player again from inside the save projection. That
  projection already runs with the canonical map mutex held — it is handed the `&Player` — and
  `with_owned_player_like_cpp` locks the same mutex, so sixteen test threads deadlocked on it.
  The aura rows are now built from the `Player` the projection was given; the session-based
  wrapper is kept for callers that do not hold the lock, and says so.

- [x] **D-H19 The kill-credit path applied none of the three gates C++ puts in front of
  objective progress.** Opened and implemented 2026-10-01. The plan named one of them; reading
  the whole operation found that `Player::UpdateQuestObjectiveProgress`
  (`Entities/Player/Player.cpp:16631-16772`) refuses a matched objective for three separate
  reasons before it touches progress, and the session-side path that credits kills, talk-to,
  gameobject use and player kills checked **none** of them. Only the item path, which is a
  second implementation of the same C++ function, applied one.

  The three, in C++ order:

  1. **The raid gate** (`:16644-16646`): unless `QuestObjective::CanAlwaysBeProgressedInRaid`
     (`Quests/QuestDef.h:489-507`, eight types that are not earned by being somewhere or
     killing something), a raid group blocks the objective for a quest that is not
     `Quest::IsAllowedInRaid` (`Quests/QuestDef.cpp:511-549`: the `QuestInfoID` raid arms, then
     `QUEST_FLAGS_RAID_GROUP_OK`, then the `Quests.IgnoreRaid` config). This is the classic
     rule that a raid group cannot do ordinary quests, and it was absent.
  2. **`IsQuestObjectiveCompletable`** (`:16650-16651`), which owns the sequenced and
     progress-bar ordering. A kill could credit an objective whose predecessor was unfinished.
  3. **`QUEST_FLAGS_EX_NO_CREDIT_FOR_PROXY`** (`:16653-16655`): a `QUEST_OBJECTIVE_MONSTER`
     credit carrying an empty victim GUID is refused. That empty GUID is precisely how
     `Player::KilledMonster` (`:16568-16570`) marks the credit it grants for a
     `CreatureTemplate::KillCredit` proxy rather than for the unit that died — so the flag is
     the only thing that distinguishes the two, and the D-H4 repair that added the proxy
     expansion left it unread.

  All three now live in one pure rule applied where C++ applies them, so the gate order is
  stated once. `Quests.IgnoreRaid` is wired from the config registry through the composition
  root; its row in `cpp-world-config-registry.tsv` moves from `missing_in_rust` to its real
  consumer.

  **The proxy half is latent on this installation, and recorded as such rather than as proven
  in play.** Of 8,543 `quest_template` rows, 31 carry any `FlagsEx` at all and the only two
  values present are `8` and `0x40000000`; **no quest here carries `0x4000`**, so none of the
  2,734 `QUEST_OBJECTIVE_MONSTER` objectives could exercise it and no live run distinguishes
  before from after. Both directions are covered by tests instead.

  **The raid half has no live evidence yet**, because blocking it needs two accounts in a group
  converted to a raid. What was checked instead is that it cannot change solo or party play:
  `GROUP_FLAG_RAID` is `0x002` in both cores, and the only writer in this port is
  `Group::convert_to_raid_like_cpp`, so an ordinary party never sets it.

  **Live regression, 2026-10-01** — the point of which is that three new refusals were added to
  a working credit path. With the quest reset to incomplete, one kill of entry 721 published
  `SMSG_QUEST_UPDATE_ADD_CREDIT` and persisted `character_queststatus_objectives` `(14106, 0) = 1`
  across the clean logout; a kill of entry 94 in the same session still paid 44 XP, 8 copper and
  two looted items. Reproduce with `--loot-after-kill --melee-creature-entry 721`.

  **Named boundary:** the raid gate reads the difficulty this port resolves for the player's
  current map, which for a continent is `DIFFICULTY_NONE` exactly as C++ `Map::GetDifficultyID()`
  is. A downscaled or locked instance whose own spawn mode differs from the player's selection is
  not tracked separately, and is written at the call site.

- [x] **D-H21 Creature resistances existed in the database and nowhere else, so no spell was ever
  resisted.** Implemented 2026-10-01 as the resist half of D-H3. The installed world database has
  1,606 `creature_template_resistance` rows across 786 creatures — a Kobold Miner in Elwynn has 21
  fire resistance — and **not one of them was loaded**. `creature_template_resistance` had no
  query, no store, no field on the template record and no value on the live creature, and
  `Unit::CalcSpellResistedDamage` had no equivalent at all, so every spell hit landed in full and
  the combat log reported `resisted = 0` always.

  Ported as the chain C++ runs, in the layer that owns each part:

  * `ObjectMgr::LoadCreatureTemplateResistances` (`Globals/ObjectMgr.cpp:536-570`) as an apply
    step **onto the already-loaded templates**, which is what C++ does rather than building a
    second store, including its two rejections: a row for the physical school, and a row for a
    school at or past `MAX_SPELL_SCHOOL`.
  * `Creature::UpdateEntry`'s seeding of `UNIT_MOD_RESISTANCE_*` from the template
    (`Entities/Creature/Creature.cpp:694-699`) at spawn, beside the sparring application.
  * `Unit::GetResistance(SpellSchoolMask)` (`Entities/Unit/Unit.cpp:13982-13993`), which returns
    the **smallest** resistance among the schools in the mask — a detail easy to get backwards.
  * `Unit::CalculateAverageResistReduction` (`:2035-2077`): the caster's target-resistance aura
    and spell penetration, holy ignoring template values, the level-based term with level 20 as
    the floor for both sides, and the level-83 boss constant of 510 instead of `level * 5`.
  * `Unit::CalcSpellResistedDamage` (`:1970-2003`): the magic-only gate, the holy-on-NPCs-only
    gate, both forms of the eleven-bucket discrete probability table, the `rand_norm()` bucket
    draw, the resisted tenths, and the ignore-resistance percentage capped at 100.
  * the publication: `damage` after the resist, `originalDamage` before it (C++ assigns it
    between the critical arm and `CalcAbsorbResist`, `:1346-1347`), `resist` on the wire, and the
    `HITINFO_FULL_RESIST`/`PARTIAL_RESIST` bit on the server-side `HitInfo`.

  **A wire detail worth recording, because it looks like a bug and is not.** Those two resist
  bits are `0x80` and `0x100`, and C++ writes `SpellNonMeleeDamageLog::Flags` in **seven bits**
  (`Server/Packets/CombatLogPackets.cpp:39`). So C++ sets them on the server and then truncates
  them off the packet: the client learns about a resist from the `Resisted` field, never from the
  flags. This port now does exactly the same, and the scenario asserts the truncation rather than
  asserting a flag the target build does not send.

  **Named boundaries**, each a fact this port does not carry rather than a choice:

  * `SPELL_ATTR0_CU_BINARY_SPELL` is taken as unset, so the level-based resistance always
    applies. That is correct for the plain direct-damage spells this path serves, but the
    attribute's own rule (`Spells/SpellMgr.cpp:3470-3520` plus the trigger pass at `:3608-3640`)
    is not ported.
  * the two ignore-resistance aura families and the Chaos Bolt family exception are zero.
  * a school mask carrying both normal and magic does not get C++'s
    `min(resisted, armourReduction)` comparison (`:2021-2028`), because this port does not run
    the load-time pass that strips the normal school and records
    `SPELL_ATTR0_CU_SCHOOLMASK_NORMAL_WITH_MAGIC`.
  * the caster's target-resistance term reads the port's single aggregated
    `mod_target_resistance` rather than a per-school aura sum.

  **Live evidence, 2026-10-01, for the data path only and said so plainly.** The server applied
  **1,606 of 1,606** `creature_template_resistance` rows against 30,018 loaded templates, so every
  row found its template and a valid school; before this change the table was never read. A melee
  kill in the same session still paid 44 XP, 12 copper and a looted item, which is the regression
  that matters because every creature spawn now seeds resistances.

  **The resist roll is proven live as of 2026-10-01**, on the caster-class character the plan
  called for rather than another fixture on the warrior. A human mage was provisioned on the QA
  account, and one Fireball at a Kobold Tunneler — entry 475, which carries 21 fire resistance in
  `creature_template_resistance` — published
  `damage=11 original=13 resisted=2 absorbed=0 school=0x04 flags=0x00`. That is the formula
  exactly: the average reduction is `21 / (21 + 100) = 0.174`, whose discrete table puts the
  weight on the one- and two-tenth buckets, and `13 * 2/10` truncates to the published 2. The
  server's own trace for the same cast reads `Dealt damage to creature ... damage=11`. The scenario
  tests keep both pinned outcomes beside it. Reproduce with
  `--spell-damage 133 --spell-damage-entry 475 --spell-damage-character 6 --spell-damage-casts 1`.

- [x] **D-H22 withdrawn in part: an empty `character_spell` is faithful, and the diagnosis behind
  it was wrong.** Raised and corrected 2026-10-01, the same day, with the code change it motivated
  reverted before publication.

  The observation stands: a freshly created human mage has **zero** `character_spell` rows, and
  this port does not write the spells its skills reward. What was wrong was calling that a defect.
  C++ `Player::_SaveSpells` (`Entities/Player/Player.cpp:20647-20699`) inserts a row only for a
  **non-dependent** new or changed spell — `// add only changed/new not dependent spells` — and
  `LearnSkillRewardedSpells` learns through `LearnSpell(ability->Spell, /*dependent*/ true)`
  (`:24186`), so C++ never writes those rows either. They are recomputed from `character_skills`
  at every login, which is exactly what this port does: the mage's 11 default skills **are**
  persisted, and the login log reports `loaded_skill_count=11 ... total_spell_count=43` on every
  later login. Reading the bare row count as "the character knows nothing" was the error.

  **A second claim in the first draft was also wrong and is worth keeping.** It said the DB2 side
  of `LearnDefaultSkills` is not walked, citing `default_skill_count=0`. That figure is zero on a
  *later* login because the skills are already known and C++ skips a known skill
  (`:23990-23993`). On the character's **first** login the same line reads
  `default_skill_count=11`, so the walk works.

  The change this drove — marking a login-learned spell `New` rather than `Unchanged` in the
  post-login spell map — was reverted. C++'s state does follow `learning` rather than loading
  (`AddSpell`, `:2698`), so the port's `Unchanged` is unfaithful in principle, but every spell the
  observed path learns is dependent and therefore never written, so the only observable effect
  would have been an extra favourite-row delete per spell per save. A behaviour change in the save
  path needs a case where it matters, and this evidence does not supply one.

  **The remaining question is answered, 2026-10-01: the grant is correct.** A new
  `RUSTYCORE_KNOWN_SPELLS_TRACE` logs the ids on `SMSG_SEND_KNOWN_SPELLS`, and a level-20 human
  mage is granted 43 spells including **116 (Frostbolt)** and **133 (Fireball)** — the two the
  class needs and the first of which `playercreateinfo_action` puts on action button 0. So nothing
  about the caster lane was broken.

  **Proven by removing the fixture rather than by reading a log.** With `character_spell` emptied
  to zero rows and the bot's spellbook seeding turned off, the mage cast Fireball and the server
  published `damage=12 original=13 resisted=1 absorbed=0 school=0x04 flags=0x00`. A character with
  no rows in that table casts its class spells, which is the whole point of the dependent-spell
  rule. The `--spell-damage` mode's seeding is now opt-in (`--spell-damage-seed-spell`), because
  inserting a row there for a dependent spell writes one the target build never writes; the row
  earlier runs of this session created was removed.

- [x] **D-H23 withdrawn: the mage was out of mana, and the server said so once the harness could
  read it.** Raised and withdrawn 2026-10-01, the same day, with nothing changed in the server.

  The draft claimed a player gets one spell cast per session, from four requests producing one
  execution and no refusals. Three measurements took that apart, in this order:

  1. **The handler receives every request.** With
     `RUST_LOG=wow_world::handlers::spell=debug`, all three `CMSG_CAST_SPELL` appear with their
     own `cast_id`, so nothing is lost in transport or dispatch.
  2. **Admission accepts every request.** A trace on the cast gate reports
     `remaining=Some((0, 0))` for all three, so neither the global cooldown nor an active cast
     holds them, and the retained-active-cast theory in the draft was wrong.
  3. **The refusals were `SPELL_FAILED_NO_POWER`.** Once the harness reported the real
     `SpellCastResult` — it had been printing every refusal as `SPELL_CAST_OK` because of a
     packed-field slip — a three-cast run came back `[108, 108, 108]`, and 108 is
     `SPELL_FAILED_NO_POWER` (`Miscellaneous/SharedDefines.h:1574`).

  The mage simply runs out of mana. A session starts with the mana saved at the previous logout,
  the previous run had spent it, so it gets about one cast and the server correctly refuses the
  rest. The runs that looked silent were the harness's read window plus the queued-request path,
  not a dropped request.

  **The world-pass warning was never the cause either, and the measurement says so with numbers.**
  In the session that produced the three casts the coordinator waited past its deadline 19 times,
  median 11 ms and maximum 192 ms, with 53 synchronous database queries inside world ticks. That is
  the ten-millisecond budget being exceeded by milliseconds, not a pass taking seconds.

  **What this leaves is a harness gap, not a server defect:** a sampling run needs a mana fixture
  (`characters.power1`) in the same place the mode already revives a dead character, since C++'s own
  `SPELL_FAILED_NO_POWER` is what stops a drained caster. That fixture now exists.

  **The last piece of the draft's story also dissolved, with the fixture in place.** A twenty-cast
  run produced `casts_sent=20 refusals=0`, and the server executed **16** of them
  (`Executing spell effect ... spell_id=133` ×16) while only **4** reached
  `Dealt damage to creature`, for 10, 14, 11 and 11 damage. Those four add to 46, which is about a
  Kobold Tunneler's health: the target died and the rest of the casts hit a corpse, which C++ also
  refuses (`EffectSchoolDMG` requires a living target). So "the casts vanish" was a dead creature,
  not a dropped request. One real harness residue remains, recorded rather than rounded off: the bot
  captured 2 of the 4 published damage logs, so its drain loop still misses some.

  The three traces added while measuring this are kept: the cast-admission pair, the two reasons a
  pending request is held, and the residence-revision drop. Each one turns a silent branch into a line, which is what made the
  difference here.

- [x] **A harness defect worth recording beside it: the refusal reader reported every
  `SMSG_CAST_FAILED` as success.** `SpellCastVisual` serialises **one** `uint32` on this branch —
  `ScriptVisualID` is commented out in the C++ and the port's writer matches
  (`wow-packet/src/packets/spell.rs:227-229`) — so reading two put the `SpellCastResult` four
  bytes late and returned `FailedArg1`, which is zero, i.e. `SPELL_CAST_OK`. A refusal that reads
  as success is worse than one that reads as garbage, so the fix is pinned by a test that places
  the reason after exactly one visual field. This is the second packed-field slip in this harness
  this session; both were caught by reading the server's own writer rather than by guessing.

- [x] **D-H24 The absorb loop's ignore-absorb term was invented: a per-shield test, and a spell
  attribute the 3.4.3 server never reads.** Found on 2026-10-02 while reading
  `Unit::CalcAbsorbResist` for the spell side of D-H3, and fixed in the same reading.

  C++ takes the attacker's `SPELL_AURA_MOD_TARGET_ABSORB_SCHOOL` share out of the damage **once**,
  before both shield loops, and puts it back **once** after them:
  `absorbIgnoringDamage = CalculatePct(damageInfo.GetDamage(), auraAbsorbMod)` at `Unit.cpp:2106`,
  `damageInfo.ModifyDamage(-absorbIgnoringDamage)` at `:2112`, and
  `damageInfo.ModifyDamage(absorbIgnoringDamage)` at `:2250`. Inside the loops the shields only ever
  read `damageInfo.GetDamage()`; there is no exemption test of any kind. The port instead subtracted
  the share from **each** shield's cap inside both loops and never restored it, and gated that
  subtraction on `SPELL_ATTR6_ABSORB_CANNOT_BE_IGNORE`. That attribute is declared in the reference
  (`SharedDefines.h:697`, `enuminfo_SharedDefines.cpp:1042`) and **read nowhere in the server** — a
  tree-wide search finds only the declaration and its reflection table — so no shield is exempt in
  3.4.3, and `cannot_be_ignored` was state with no source.

  Two observable consequences, both now gone: with the modifier active the final damage was short by
  the ignored share (the port absorbed from the reduced damage and never added the share back), and
  a shield carrying the attribute absorbed a whole hit that C++ would have let partly through. The
  per-shield subtraction also compounded across shields.

  The repair moves the term to where C++ keeps it. `represented_absorb_stages_like_cpp`
  (`session_rules/rules_4.rs`) is now the single owner of C++'s absorb half: it holds the share out,
  runs the school-absorb loop and then the mana-shield loop over what is left, and adds the share
  back. Neither loop takes an ignore argument any more, and `cannot_be_ignored` is removed from both
  shield projections. One more fidelity detail came out of the same reading and is now kept: C++
  reads the percentage from the damage **before** `ResistDamage` (`:2106` precedes `:2111`), so the
  composition takes both the pre-resist and post-resist damage; for a physical melee hit they are the
  same value, because `CalcSpellResistedDamage` returns zero for a non-magic school mask
  (`Unit.cpp:1972-1974`).

  Evidence: `represented_ignore_absorb_matches_calc_absorb_resist_like_cpp` replaces the test that
  asserted the invented shape and now pins the hold-out, the restore, the pre-resist basis, the
  100%-ignored case and both loops in sequence;
  `legacy_creature_melee_tick_once_honors_ignore_absorb_like_cpp` was likewise asserting the
  exemption and now asserts that the attribute changes nothing.

  One circumstantial detail, recorded as a lead rather than a conclusion: the comments carrying the
  invented term also carried line numbers hundreds of lines away from the 3.4.3 functions they
  named (`Unit.cpp:1791-1880` for a loop that lives at `:2114-2178`). The anchors therefore did not
  come from the pinned reference. Which file they did come from is not established here — the
  complementary 3.3.5a checkout is not present on this host, so that is a question for whoever has
  it, not a claim. Corrected anchors are in the same commit.

- [x] **D-H25 A creature's aura was written to the mirror that gets overwritten, so one no-op
  mutation erased it.** Found and fixed on 2026-10-02 while implementing D-H3's absorb stage, which
  could not see a shield that no longer existed.

  A creature can exist twice: as a legacy-runtime `WorldCreature` and as a canonical map entity.
  `mutate_world_creature` ends by calling `sync_canonical_creature_entity_like_cpp`, which replaces
  the **whole** canonical entity from the legacy clone — deliberately, because death and respawn
  hooks touch AI, combat, loot, aura, timer and plan state together
  (`session/mod.rs:4013-4021`). `apply_creature_aura_with_provenance_like_cpp` wrote the application
  only to the canonical side. So any legacy mutation of that creature discarded it, and a no-op one
  was enough.

  Measured rather than reasoned about: a probe applied a 300-point shield, read `Some(300)`, called
  `mutate_world_creature(guid, |_| {})`, and read `None`. The spell-hit path mutates the legacy
  creature twice before any shield is read — once to read resistance and level for the resist, once
  to apply the damage — so the shield was always gone by then. In play this reached every creature
  aura a player cast applied (`spell_effects/execution.rs:853`): a debuff landed, published its slot
  to the client, and vanished at the victim's next swing or hit with no packet saying so.

  The repair gives the aura state one owner per creature. `mutate_creature_aura_owner_like_cpp`
  writes the legacy mirror when the creature is registered there — the sync then carries the state
  to canonical, as it does for health — and the canonical entity directly otherwise, which is what a
  summon or pet is. The registration body moved into
  `register_creature_aura_application_like_cpp(&mut Creature, …)` so both mirrors share one
  implementation rather than two aura tables, and the slot lookup, the expiry removal and the new
  absorb stage all go through the same owner.

  Evidence: `a_creature_aura_survives_a_legacy_mirror_mutation_like_cpp` is the probe turned into a
  regression, and the four absorb scenarios would all fail without this. Boundary: this fixes the
  aura table's owner. Whether any **other** canonical-only creature write has the same exposure is
  not audited here; the sync's whole-entity replacement is unchanged and still the thing to check
  before writing canonical-only creature state.

- [x] **D-H26 Every spell effect was worth its raw `EffectBasePoints`, so no spell ever rolled its
  damage range and no spell scaled with the caster's level.** Found on 2026-10-02 while sizing the
  creature-spell macro, and fixed in the same pass.

  C++ never hands an effect handler the DB2 column. `Spell::EffectHandler` fills `damage` from
  `SpellEffectInfo::CalcValue(caster)` (`Spells/SpellInfo.cpp:496-597`), so by the time any handler
  runs, the value already includes the `DieSides` roll (`:519-526`) and the `RealPointsPerLevel` term
  with its `MaxLevel`/`BaseLevel` clamp and `max(BaseLevel, SpellLevel)` subtraction (`:506-517`).
  The port's execution path read `effect.effect_base_points` straight out of the effect
  (`session/spell_effects/execution.rs`), for **every** effect kind — damage, heal, and every other
  handler fed from that tuple.

  Two consequences, both systematic rather than occasional. Every spell with a damage or healing
  *range* delivered the bottom of it, always, because the range lives entirely in `DieSides`. And a
  spell whose value grows with the caster's level never grew: a level-20 caster got the level-1
  value.

  Measured on the installed client data rather than estimated, with the port's own DB2 reader:
  **48,102 of 69,504** `SpellEffect.db2` rows carry a non-zero `DieSides`, and **2,117** carry a
  non-zero `RealPointsPerLevel`, of which **701** are `SPELL_EFFECT_SCHOOL_DAMAGE`. `SpellLevels.db2`
  has 16,914 rows, 16,878 of them with a non-zero `BaseLevel` or `SpellLevel`, so the clamp the
  per-level term needs has data for essentially every spell.

  The live run from 2026-10-01 already contained the symptom, which is worth recording because it was
  looked at and not noticed: the captured non-critical row reported `original_damage = 13`, and the
  critical row's 19 is `13 + 13/2` truncated, so both casts started from the same 13 where a die range
  should have varied. Two casts are not proof on their own; the source is.

  The repair ports the missing arms. `SpellEffectInfo::calc_value_with_caster_and_die_roll_like_cpp`
  carries the level term, the die roll and the `PointsPerResource` combo term in C++'s order, and the
  existing no-caster entry point now delegates to it with both unit arms off, so the two cannot drift
  apart. `effect_real_points_per_level` and `effect_points_per_resource` were already read into
  `SpellEffectDb2Entry` and never carried into the runtime effect; they are now. `SpellLevels.db2`
  was loaded and keyed but never reached the spell store either, and now lands in a side table with
  the same difficulty-fallback walk the spell-hit metadata uses — beside the runtime `SpellInfo`
  rather than on it, because that struct is named field-by-field in roughly 300 fixtures and
  `CalcValue` is the only consumer.

  The creature-level multiplication (`:541-594`) is in too, and a correction belongs here because I
  first wrote it off as unreachable: I had looked only inside `dbc/<locale>/` and concluded the
  installed data shipped no GameTable files and the port had no reader. Both were wrong.
  `/opt/wow-3.4.3/gt/NPCManaCostScaler.txt` is installed, and `wow-data::game_tables` already reads
  several GameTables. So `NpcManaCostScalerGameTableLikeCpp` is a small addition in the shape of its
  neighbours, and `value *= casterScaler->Scaler / spellScaler->Scaler` now runs. The gate predicate
  stays public anyway, because a caller holding no table still needs to know when C++ would have
  scaled.

  Boundaries that remain, each a fact this port does not carry rather than a choice: combo points are
  zero because no owner tracks `Unit::GetComboPoints`, which leaves the `PointsPerResource` term inert
  for the 66 effects that have one, and `ApplyEffectModifiers`'s spellmods have no represented owner.
  The player's own cast path passes no scaler table, which is not a gap either: C++ gates that arm on
  `!IsControlledByPlayer()` (`:544`), so a player caster cannot reach it. `SpellLevels` also takes the plain DB2 load
  rather than a hotfix overlay, because that table has no overlay path here; a `spell_levels` hotfix
  row would not apply. The core spell loader now owns that store and hands it back, so the file is
  still read once and `app.rs` ends six lines *below* its physical ceiling rather than one above it.

  Evidence: `calc_value_with_caster_matches_cpp_level_and_combo_arms` pins every arm including the
  `MaxLevel` cap, the raise to `BaseLevel`, the negative term a `SpellLevel` above `BaseLevel`
  produces, the truncation of `int32(level * basePointsPerLevel)`, the three `DieSides` branches and
  the no-caster agreement; `calc_value_creature_level_scaling_gate_matches_cpp` pins that arm's gate
  and `calc_value_creature_level_scaling_applies_the_npc_mana_cost_scaler_like_cpp` the multiplication
  itself, with `npc_mana_cost_scaler_parses_the_installed_game_table_like_cpp` reading the real
  `NPCManaCostScaler.txt` (101 rows, level 1 at `0.193`, row 0 the unused default); and
  `a_spell_effects_damage_is_its_calc_value_not_its_base_points_like_cpp` drives a real cast end to
  end, where a 10-base-point spell with `DieSides 6` and `RealPointsPerLevel 2.0` at level 20 deals
  **52**, not 10. `cargo test -p wow-data --lib` 765 passed; `-p wow-world --lib` 4280 passed; `-p world-server --lib` 607 passed.

  **Not proven live.** The next `--spell-damage` sampling run should now show a *range* of
  `original_damage` where it showed a constant 13, and that is the acceptance owed.

## MED — wrong values / loose checks / minor loss

- [ ] **D-M1 Silent gold-save error.** `let _ = char_db.execute(stmt).await` swallows failures. `session.rs:21495`.
- [x] **D-M2 was not a defect: C++ also discards group-money division remainder.**
  `LootHandler.cpp::HandleLootMoneyOpcode` computes `loot->gold / playersNear.size()` and credits
  that same truncated amount to every recipient; there is no first-recipient remainder branch.
- [ ] **D-M3 Off-hand dual-wield damage has no penalty** (~25% too high). `session.rs:7923`.
- [ ] **D-M4 Haste has no attack-speed cap** → scales unbounded. `session.rs:1358`.
- [ ] **D-M5 Threat = raw damage**, no ability/role threat modifiers. `session.rs:47875`.
- [x] **D-M6 Equipment sets in-memory only** was closed by issue #112 / PR #113: sets and
  transmog outfits now persist transactionally and load on fresh authentication with a shared
  collision-safe GUID namespace.
- [x] **D-M7 Void storage not saved** was closed by issue #114 / PR #115 with one atomic
  flags/money/inventory/void transaction, fresh-auth lifecycle proof, legacy-backpack repair,
  deposit quest-objective persistence, live withdrawal collection updates, focused tests and a
  byte-clean C++/Rust query capture. PR #115 merged as `55719eb4` with all required gates green.
- [ ] **D-M8 Group member DB insert fail logged-only**, runtime kept → reload drops member. `handlers/group.rs:1090`.
- [ ] **D-M9 Phase not re-checked on movement** → out-of-phase objects linger. `handlers/movement.rs:274`.
- [ ] **D-M10 Position save binds extra `instance_id`** vs C++ 7-field SavePosition (verify SQL param alignment). `session.rs:21578`.
- [ ] **D-M11 Loaded-grid GameObject/AreaTrigger GUID helpers hardcode realm 0.**
  `create_gameobject_like_cpp` and `create_area_trigger_like_cpp` in
  `wow-core/src/guid.rs` are used by the typed world-server/area-trigger load
  paths with realm zero. Both C++ trees pass zero at the world-object callsite
  but `ObjectGuidFactory::CreateWorldObject` replaces it with the active
  `realm.Id.Realm`; Rust currently skips that substitution. Creature/Vehicle
  callers were corrected after the issue #81 capture exposed the same defect,
  but this separate GO/AreaTrigger boundary remains open. C++
  `ObjectGuid.cpp:590-631`; Rust `world-server/src/main.rs` and
  `area_trigger_loaded_grid.rs`.
- [ ] **D-M12 `SMSG_LOGOUT_COMPLETE` uses the instance socket before channels
  are restored.** Live C++/Rust bot QA for issue #81 observed Rust routing
  `0x2684` on instance while stock C++ routes it on realm. The immediate logout
  path calls `send_packet(&LogoutComplete)` before `restore_realm_channels()`;
  the timed path also sends through the current channel. C++
  `Opcodes.cpp:1665` (`CONNECTION_TYPE_REALM`); Rust
  `handlers/character.rs::handle_logout_request` and
  `session.rs::complete_logout`. Functional relog succeeds because both bot
  sockets remain open, but wire routing is not parity-clean.
- [ ] **D-M13 Base `AreaTable.db2` loader reads four physical fields one
  position late.** `AreaTableMeta` uses an external ID (`IndexField = -1`), so
  the WDC4 indices for `ContinentID`, `ParentAreaID`, `AreaBit`, and
  `ExplorationLevel` are respectively `2`, `3`, `4`, and `11`; Rust currently
  reads `3`, `4`, `5`, and `12`. Hotfix rows use the C++ `DB2LoadInfo`/SQL
  column ordinals including ID and are not affected. The issue #81 review
  verified that the newly used `FactionGroupMask` index `14` is already
  correct (hotfix column `15`), as are `MountFlags` `16` and `Flags1` `21`.
  C++ `DB2Metadata.h::AreaTableMeta` / `DB2LoadInfo.h::AreaTableLoadInfo`;
  Rust `wow-data/src/area.rs::AreaTableStore::load`.
- [x] **D-M14 Effective skill relation stores used source-interleaved startup
  order instead of C++ table-granular order.** Rust loads both
  `SkillLineAbility` and `SkillRaceClassInfo` WDC4 bases, then queries ability
  official, race-class official, ability custom, race-class custom. C++
  `DB2Manager::LoadStores` completes each `LOAD_DB2` independently, and
  `DB2StorageBase::LoadFromDB` loads official then custom before advancing to
  the next table. The persistence refactor #523 intentionally preserves this
  observable pre-existing query/failure order. The bounded correction in #524
  (`020163dc`) now completes official/custom `SkillLineAbility` before querying
  official/custom `SkillRaceClassInfo`, preserving the same failure boundary.
  C++ `DB2Stores.cpp:848-850`, `DB2Store.cpp:127-133`,
  `DB2DatabaseLoader.cpp:28-33`; Rust
  `wow-database/src/hotfix/skill_catalog_adapter.rs::load_skill_relation_hotfix_rows_like_cpp`.
  The wider #524 family remains open because Rust does not yet load and consume
  `SkillLineXTraitTree` through a `TraitMgr`-equivalent production authority.

- [x] **D-M15 `QuestLogItemId` was credited and put on the wire, and the target build does
  neither. Latent, not live.** Closed 2026-10-01.

  RustyCore read `item_template_addon.QuestLogItemId`, credited `QUEST_OBJECTIVE_ITEM`
  objectives keyed on it in addition to the item entry, and wrote it into
  `SMSG_ITEM_PUSH_RESULT.QuestLogItemID`. In `/home/server/woltk-trinity-legacy` the field
  appears exactly once in the whole server, as the commented-out line
  `//packet.QuestLogItemID = item->GetTemplate()->QuestLogItemId;`
  (`Entities/Player/Player.cpp:13869`), so stock `Player::SendNewItem` ships the packet
  default for every push; and `ItemAddedQuestCheck(uint32 entry, uint32 count)`
  (`Entities/Player/Player.h:1557`, body at `:16533-16536`) hands
  `UpdateQuestObjectiveProgress` the item entry and nothing else.

  Both halves are repaired at the one point each becomes observable: the push-result
  conversion writes `0`, and the credit paths key on the item entry alone. A positive and a
  negative test pin it — an objective on the stored item's entry advances, one on the
  template's `QuestLogItemId` is left untouched — and the `SMSG_ITEM_PUSH_RESULT` mapping test
  now asserts `0` however the plan was filled.

  **It could not have shown up in play, and that is worth stating rather than dressing up.**
  All 625 rows of the installed `item_template_addon` have `QuestLogItemId = 0`, so no item on
  this installation could produce a non-zero credit id or a non-zero wire field. There is
  therefore no live run that distinguishes before from after, and none was staged. The
  evidence is the source, which is conclusive on its own, plus the capture-diff semantic rule
  for the issue-106 `ItemPushResult`, which already requires `QuestLogItemID == 0` from the
  real byte stream (`capture-diff/src/semantic/state_4.rs:358,388`).

  **Deliberately not done:** retiring the ~89 remaining references that compute and carry the
  value through the loot, item-store, void-storage and spell paths. With both decision points
  faithful and every data row zero, they are inert rather than wrong, and removing them is a
  twenty-file mechanical change with no behaviour at stake. It belongs with the next change
  that owns `item_template_addon`, not bolted onto this one. Recorded as D-L4.
- [ ] **D-L4 The inert `QuestLogItemId` plumbing should be retired.** After D-M15 the value is
  read from `item_template_addon`, cached, threaded through the loot, item-store,
  void-storage, spell and quest paths and then ignored at both points where it used to be
  observable. About 89 non-test references across twenty files carry a number nothing
  consumes, which is how the original divergence survived unnoticed. No behaviour depends on
  it, so this is cleanup to fold into the next change that owns `item_template_addon`.
- [x] **D-M16 The global player-melee phase used the boundary radius as a second range
  requirement, so a facing attacker four yards away never swung.** Closed 2026-10-01. Found
  by chasing a swing that three live runs could not land: the phase counted
  `creature_hits=1 commands=1 delivered=1 queued=1` while the bot saw no
  `SMSG_ATTACKER_STATE_UPDATE` on either socket and the creature survived. Two things were
  wrong, one of them in this file's own earlier wording of the symptom.

  The command reached the session and was accepted; it simply carried no swing
  (`gate="accepted" swings=0`, 38 times in one run). `outcome.creature_hits` was
  incremented for every result the phase got back from the creature, including a result
  with an empty swing list, so the trace claimed hits the victim never took. That counter
  now only counts a swing that exists.

  The swing itself was refused with `AttackSwingErr::NotInRange` at a measured 4.00 yards
  (character `(-9159.58, 81.79, 77.45)`, `creature.guid = 280092` at
  `(-9162.37, 84.63, 77.08)`), with facing true. C++
  `Unit::DoMeleeAttackIfReady`'s `getAutoAttackError`
  (`Entities/Unit/Unit.cpp:2447-2459`) asks two independent questions:
  `!IsWithinMeleeRange(victim, IsPlayer())` is `NotInRange`, and
  `!IsWithinBoundaryRadius(victim) && !HasInArc(2*pi/3, victim)` is `BadFacing`. The
  boundary radius (`Unit::IsWithinBoundaryRadius`, `:806-814`) therefore **exempts** a very
  close attacker from the facing arc; it is not a second range test.
  `legacy_runtime/player_tick.rs` had it as
  `in_melee_range = IsWithinMeleeRange && IsWithinBoundaryRadius`, which turns the
  exemption into a requirement and reports the wrong error code. Since the runtime combat
  reaches are zero here, the boundary term evaluated to `2.0` and refused every swing
  beyond two yards, while `GetMeleeRange`'s `NOMINAL_MELEE_RANGE` floor of `5.0` hid the
  problem from the other half of the condition.

  The same C++ shape was already correct in the two sibling implementations — the session
  path (`session/spell_effects/ticks.rs`, `boundary || facing`) and the creature tick
  (`legacy_runtime/creature_movement_tick.rs`, `!boundary && !facing => BadFacing`) — so
  this was one site diverging from its own neighbours, not a missing port.
  `Unit::DoMeleeAttackIfReady`'s boundary call is `alistar:`-patched out in the pinned
  reference, so the stock shape was taken from the surrounding code and the untouched
  `IsWithinBoundaryRadius`/`AttackSwingErr` definitions, not from the patched line.

  Live, after: one 90-second run killed the target — `player_landed=4 (45 damage)`,
  `death=true`, `xp=44`, `loot_coins=9`, `money 12 -> 21`, `inv 8 -> 10`. Reproduce with
  `--loot-after-kill --melee-creature-entry 94 --melee-creature-guid 280092`.

  **Still open from the same investigation:** runtime `combat_reach` and `bounding_radius`
  are zero for the player and for this creature, which is what made the boundary term so
  small. C++ sets them from the model (`Creature::SetObjectScale`, and the player's
  `DEFAULT_PLAYER_COMBAT_REACH` by scale), and `Unit::GetMeleeRange`'s `5.0` floor hides
  the difference for melee but not for the other distance checks that read the same
  fields. Not repaired here; recorded as D-M17.
- [x] **D-M17 A logged-in player had no `BoundingRadius` and no `CombatReach` at all.**
  Closed 2026-10-01. Found while closing D-M16, which the zero reach had made worse.

  C++ `Player::SetObjectScale` (`Entities/Player/Player.cpp:1582-1586`) is the only writer
  of those two fields for a player — `scale * DEFAULT_PLAYER_BOUNDING_RADIUS` and
  `scale * DEFAULT_PLAYER_COMBAT_REACH` (`Entities/Object/ObjectDefines.h:39-40`) — and it
  is called at `Player::Create` (`:439`), when resetting stats before reapplying auras
  (`:2312`) and at `Player::LoadFromDB` (`:17645`). RustyCore had **no** writer: no
  production path called `Unit::set_combat_reach` for a player, so both fields stayed at
  their `0.0` default for the whole session. The creature side was fine — the production
  create paths derive both from `CreatureModelInfo` by scale
  (`session/world_entities/creature.rs:281-282`), and the hard-coded `0.389 / 1.5` in
  `map_manager/runtime/creature.rs:83-84` belongs to `WorldCreature::new`, which has no
  production caller.

  It was invisible in two ways at once. The player's CREATE block wrote the correct
  literals straight into the packet
  (`wow-packet/.../update/player/state_2.rs`), so the client always saw `0.389 / 1.5` and
  only the server disagreed; and `player_interaction_combat_reach_like_cpp` substituted
  `DEFAULT_PLAYER_COMBAT_REACH` whenever it read a zero, which quietly fixed the one
  consumer that would have shown it. Every other reader of the field was simply wrong:
  `Unit::IsWithinBoundaryRadius`'s radius lost `1.5` yards, as did
  `WorldObject::_IsWithinDist`'s combat-reach term. `Unit::GetMeleeRange`'s
  `NOMINAL_MELEE_RANGE` floor of `5.0` absorbed the loss for melee range itself, which is
  why nothing had failed outright before D-M16 turned the boundary radius into a range gate.

  The repair adds `Player::set_object_scale_like_cpp` as the port of
  `Player::SetObjectScale` and calls it at the login bootstrap, removes the substituting
  fallback so a future zero is visible instead of patched, and gives the two
  `ObjectDefines.h` values a single home in `wow_constants::object` (the copy in
  `wow-map` stays, with the reason: that crate does not depend on `wow-constants` and one
  float is not worth a new crate edge). The CREATE block now writes those constants instead
  of magic numbers; a *scaled* player would still need the entity's own values there, which
  `PlayerCreateData` does not carry and no RustyCore path needs yet.

  Evidence: two entity tests on the derived values and their scaling, and a login-bootstrap
  test that the canonical player comes out of `build_initial_player_for_owner_like_cpp` with
  `0.389 / 1.5`. One existing test changed behaviour and was corrected rather than
  re-baselined: `combat_tick_bad_facing_sets_short_retry_timer_like_cpp` placed its victim
  at 2.0 yards, which was outside the boundary radius only while the player's reach was
  zero; with the C++ value the attacker is inside it, and C++ then exempts the facing arc,
  so the fixture now stands at 4.0 yards — outside the `3.5` boundary and inside the `5.0`
  melee range, which is the only band where bad facing is what refuses the swing. That
  behaviour change is itself the integration evidence that the field now reaches the
  predicate. The live run after the repair is a regression check, not a measurement: the
  wire carried the right literal either way, so no capture can distinguish the two.
  `--loot-after-kill --melee-creature-entry 94 --melee-creature-guid 280092` still reports
  `player_landed=4 (43 damage) creature_landed=3 death=true xp=44 loot_coins=8`,
  money 21 -> 29.

## LOW — non-issues in practice / cosmetic (recorded for completeness)

- [ ] **D-L1 Item StackCount/Durability written `i32`** vs C++ `uint32` — identical bytes for
  realistic values; only wraps >2³¹. `update.rs:5271,5294`. Tidy, not urgent.
- [ ] **D-L2 Item Expiration/Artifact size fields** type/empty-array cosmetics. `update.rs:5272,5307`.
- [ ] **D-L3 DK DisplayPower=5 (Runes) vs 6 (RunicPower)** — also tracked as #1213/M1.4. `update.rs:1793`.

---

## How this feeds the plan

These are **bugs in shipped code**, distinct from missing features. In `PORT_PLAN.md` they are
the **D-track** (existing-code hardening), checkbox-tracked here. Priority placement:
- **CRIT data-loss/dupe (D-C1..C9)** → pulled into **M0/M1** (we can't validate gameplay on a
  server that loses enchants, wipes banks, or dupes loot).
- **Combat correctness (D-H1..H3)** → folded into **M3** (real combat) — they're why M3 exists.
- **Quest crediting (D-H4..H6)** → **M4.7**.
- The rest → addressed in their owning milestone, verified by capture/round-trip test.
