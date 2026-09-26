//! SQLx-free contracts of the worldserver Battle.net (BattleTag) friends manager.
//!
//! LegionCore 7.3.5 keeps `Battlenet::FriendsMgr` in the worldserver, loaded once
//! at startup (`LoadFromDB`: ">> Loaded %u Battle.net friend links and %u pending
//! invitations in %u ms") from the auth tables `battlenet_account_friends`
//! (two rows per friendship, `PK(account_id, friend_id)`) and
//! `battlenet_account_friend_invitations` (`id BIGINT PK, inviter_id, invitee_id,
//! message, created, role`), and resolves invitation targets by
//! `battlenet_accounts.battle_tag` or `email`. TrinityCore 3.4.3 has none of this
//! (`FriendsService` / `PresenceService` are `ERROR_RPC_NOT_IMPLEMENTED` stubs), so
//! the SQL shape is LegionCore's, carried by
//! `sql/updates/auth/wotlk_classic/2026_09_26_00_auth.sql`.
//!
//! Every write that changes a friendship commits both directions in one Login DB
//! transaction (`accept_invitation_like_cpp`: two link rows + the consumed
//! invitation; `delete_friendship_like_cpp`: both rows) so a half friendship
//! cannot exist. The manager mutates its in-memory state only after `Applied`.

use crate::{PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp};

/// `battlenet_accounts(id, email, battle_tag)`: what the friends manager knows
/// about an account (invitation names, BattleTag lookups).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BnetAccountIdentityLikeCpp {
    pub account_id: u32,
    pub email: String,
    /// `battlenet_accounts.battle_tag` (`Name#1234`); empty when NULL.
    pub battle_tag: String,
}

/// One `battlenet_account_friends` row (one direction of a friendship).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BnetFriendLinkRowLikeCpp {
    pub account_id: u32,
    pub friend_id: u32,
    /// The note `account_id` keeps about `friend_id` (`FriendsService.UpdateFriendState`).
    pub note: String,
    /// Friend role id (1 = BattleTag friend).
    pub role: u32,
}

/// One `battlenet_account_friend_invitations` row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BnetFriendInvitationRowLikeCpp {
    pub id: u64,
    pub inviter_id: u32,
    pub invitee_id: u32,
    pub message: String,
    /// `created` as a unix timestamp.
    pub created: u64,
    pub role: u32,
}

/// LegionCore `FriendsMgr::LoadFromDB` result.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BnetFriendsLoadLikeCpp {
    /// Every `battlenet_accounts` row (id, email, battle_tag) so friend and
    /// invitation names resolve without a query per session.
    pub accounts: Vec<BnetAccountIdentityLikeCpp>,
    pub links: Vec<BnetFriendLinkRowLikeCpp>,
    pub invitations: Vec<BnetFriendInvitationRowLikeCpp>,
}

/// How `SendInvitation` and a late-registered session resolve an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BnetAccountLookupLikeCpp {
    Id(u32),
    /// `FriendInvitationParams.target_battle_tag`.
    BattleTag(String),
    /// `FriendInvitationParams.target_email`.
    Email(String),
}

/// Login database capability of the Battle.net friends manager.
pub trait BnetFriendsPersistencePortLikeCpp: Send + Sync {
    /// LegionCore `FriendsMgr::LoadFromDB`: every account identity, friend link
    /// and pending invitation.
    fn load_all_like_cpp(
        &self,
    ) -> PersistenceFutureLikeCpp<'_, Result<BnetFriendsLoadLikeCpp, String>>;

    /// `battlenet_accounts` lookup for an account the startup load did not see
    /// (created since) or an invitation target.
    fn find_account_like_cpp(
        &self,
        lookup: BnetAccountLookupLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BnetAccountIdentityLikeCpp>, String>>;

    /// `INSERT INTO battlenet_account_friend_invitations`.
    fn insert_invitation_like_cpp(
        &self,
        invitation: BnetFriendInvitationRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// Decline / revoke / ignore: `DELETE` the invitation row.
    fn delete_invitation_like_cpp(
        &self,
        invitation_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// Accept: both `battlenet_account_friends` rows inserted and the invitation
    /// deleted, one transaction.
    fn accept_invitation_like_cpp(
        &self,
        invitation: BnetFriendInvitationRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// `RemoveFriend`: both rows deleted, one transaction.
    fn delete_friendship_like_cpp(
        &self,
        account_id: u32,
        friend_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;

    /// `UpdateFriendState` note attribute: `UPDATE ... SET note = ?` of the
    /// `(account_id, friend_id)` row.
    fn update_friend_note_like_cpp(
        &self,
        account_id: u32,
        friend_id: u32,
        note: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp>;
}
