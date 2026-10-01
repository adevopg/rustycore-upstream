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

- [ ] **D-H1 Melee damage has no formula.** Uses raw weapon-damage range as final damage; **no
  armor mitigation, no AP scaling, no level reduction.** `session.rs:7913-7942`. C++
  `Unit::CalcArmorReducedDamage` / AP→damage.
- [ ] **D-H2 Melee hit table absent.** miss/dodge/parry/block/glancing/crit all bypassed;
  hardcoded `HIT_INFO_NORMAL_SWING|VICTIM_STATE_HIT`. `session.rs:47813-47823`. C++
  `Unit::MeleeSpellHitResult`.
- [ ] **D-H3 Spell damage/heal uses raw base points.** No coefficient, crit, or resist.
  `session.rs:49014-49026`.
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
