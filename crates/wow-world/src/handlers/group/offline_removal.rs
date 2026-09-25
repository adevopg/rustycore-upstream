//! C++ `Group::RemoveMember(guid)` (default method) for an offline character,
//! as `HandleCharRaceOrFactionChangeCallback` calls it when a faction change
//! leaves a group and `CONFIG_ALLOW_TWO_SIDE_INTERACTION_GROUP` is off.
//!
//! Same canonical owner and follow-up as `LeaveGroup` (`ops_1.rs`), without the
//! leaving member's own packets: that character is at the character screen.

use super::*;

impl WorldSession {
    /// Remove `member_guid` from the group stored as `db_store_id`
    /// (`sGroupMgr->GetGroupByDbStoreId`); no-op when the group is not loaded.
    pub(crate) async fn remove_offline_group_member_like_cpp(
        &self,
        db_store_id: u32,
        member_guid: ObjectGuid,
    ) {
        let (Some(group_reg), Some(registry)) = (
            self.group_registry().map(std::sync::Arc::clone),
            self.player_registry().map(std::sync::Arc::clone),
        ) else {
            return;
        };
        let Some(group) =
            wow_social::group::get_group_by_db_store_id_like_cpp(&group_reg, db_store_id)
        else {
            return;
        };
        let gid = group.group_guid;
        let connected_members = connected_group_members_like_cpp(&group, &registry);
        let Ok(outcome) = group_reg.remove_member_like_cpp(
            gid,
            member_guid,
            GroupMemberRemovalKindLikeCpp::Leave,
            &connected_members,
        ) else {
            return;
        };
        let dissolve_remaining = outcome
            .facts
            .disbanded
            .then(|| outcome.facts.remaining_members.clone());
        self.persist_group_intents_like_cpp(gid, outcome.persistence)
            .await;
        let Some(remaining) = dissolve_remaining else {
            send_party_update(&outcome.group, &registry, self.virtual_realm_address());
            return;
        };
        if let Some(&last_guid) = remaining.first() {
            if let Some(last) = registry.group_presence(last_guid) {
                let command = ApplyGroupRemovalLikeCppCommand {
                    group_guid: gid,
                    category: wow_social::group::GROUP_CATEGORY_HOME_LIKE_CPP,
                    party_type: wow_social::group::GROUP_TYPE_NONE_LIKE_CPP,
                    send_group_destroyed: true,
                    send_group_uninvite: false,
                    refresh_visible_gameobjects_or_spellclicks: true,
                };
                let _ = registry.deliver_group_state_command_like_cpp(
                    last.registration,
                    SessionCommand::ApplyGroupRemovalLikeCpp(command),
                );
            } else {
                registry.mark_group_state_reconciliation_like_cpp(last_guid);
            }
        }
    }
}
