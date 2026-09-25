//! In-game browser web tokens stored in `auth.battlenet_account_web_token`.
//!
//! Port of LegionCore `Battlenet::AuthenticationService::IssueToken`
//! (`src/server/game/Services/WorldserverService.cpp`): a token is 32 random bytes
//! rendered as 64 uppercase hex characters, inserted with
//! `LOGIN_INS_BNET_WEB_TOKEN` (`expires = NOW() + Browser.TokenLifetime`) and followed
//! by an opportunistic `LOGIN_DEL_BNET_WEB_TOKENS_EXPIRED`.
//!
//! Shared by bnet-server (`GenerateWebCredentials`, kind 0) and world-server
//! (the 3.4.3 checkout SSO token requested with `CMSG_BATTLE_PAY_OPEN_CHECKOUT`,
//! kind 1) so both write the row with the one SQL definition in
//! [`LoginStatements`]. Randomness is supplied by the caller: this crate carries no
//! RNG dependency.

use crate::{DatabaseError, LoginDatabase, LoginStatements};

/// Length of a generated token: 32 random bytes as hex.
pub const WEB_TOKEN_HEX_LEN: usize = 64;

/// `battlenet_account_web_token.program` for World of Warcraft (`WoW` FourCC).
pub const WEB_TOKEN_PROGRAM_WOW: u32 = 0x0057_6F57;

/// `battlenet_account_web_token.kind` (LegionCore comment: 0 web credentials, 1 sso).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WebTokenKind {
    /// Issued by `AuthenticationService.GenerateWebCredentials`.
    WebCredentials = 0,
    /// Issued for the checkout/support browser SSO login.
    Sso = 1,
}

impl WebTokenKind {
    pub fn from_db(kind: u8) -> Option<Self> {
        match kind {
            0 => Some(Self::WebCredentials),
            1 => Some(Self::Sso),
            _ => None,
        }
    }
}

/// Everything LegionCore's `IssueToken` writes besides the token itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebTokenIssue {
    pub battlenet_account: u32,
    /// Game account id (0 when no game account is selected yet).
    pub account: u32,
    pub realm: u32,
    /// Character GUID counter (0 outside a world session).
    pub character_guid: u64,
    /// `FourCC` from the request (`WoW` = 0x576F57), 0 when absent.
    pub program: u32,
    pub kind: WebTokenKind,
    pub ip: String,
    /// `Browser.TokenLifetime` in seconds.
    pub lifetime_secs: u32,
}

/// Render 32 bytes as 64 uppercase hex characters, like C++ `ByteArrayToHexStr`.
pub fn web_token_hex_like_cpp(bytes: &[u8; 32]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(WEB_TOKEN_HEX_LEN), |mut hex, byte| {
            let _ = write!(hex, "{byte:02X}");
            hex
        })
}

/// Insert (or refresh) `token` with the given metadata.
///
/// `INS_BNET_WEB_TOKEN` is an upsert on the `token` primary key so bnetserver can
/// re-persist an existing login ticket without a duplicate-key failure.
pub async fn insert_web_token(
    db: &LoginDatabase,
    token: &str,
    issue: &WebTokenIssue,
) -> Result<(), DatabaseError> {
    let mut stmt = db.prepare(LoginStatements::INS_BNET_WEB_TOKEN);
    stmt.set_string(0, token);
    stmt.set_u32(1, issue.battlenet_account);
    stmt.set_u32(2, issue.account);
    stmt.set_u32(3, issue.realm);
    stmt.set_u64(4, issue.character_guid);
    stmt.set_u32(5, issue.program);
    stmt.set_u8(6, issue.kind as u8);
    stmt.set_string(7, issue.ip.as_str());
    stmt.set_u32(8, issue.lifetime_secs);
    db.execute(&stmt).await?;
    Ok(())
}

/// LegionCore `IssueToken`: store the token made from `random_bytes` and purge
/// expired rows.
pub async fn issue_web_token_from_bytes(
    db: &LoginDatabase,
    issue: &WebTokenIssue,
    random_bytes: &[u8; 32],
) -> Result<String, DatabaseError> {
    let token = web_token_hex_like_cpp(random_bytes);
    insert_web_token(db, &token, issue).await?;
    purge_expired_web_tokens_best_effort(db).await;
    Ok(token)
}

/// `LOGIN_DEL_BNET_WEB_TOKENS_EXPIRED`; returns the number of rows removed.
pub async fn purge_expired_web_tokens(db: &LoginDatabase) -> Result<u64, DatabaseError> {
    let stmt = db.prepare(LoginStatements::DEL_BNET_WEB_TOKENS_EXPIRED);
    db.execute(&stmt).await
}

/// C++ fires the cleanup asynchronously and never observes its result.
pub async fn purge_expired_web_tokens_best_effort(db: &LoginDatabase) {
    if let Err(error) = purge_expired_web_tokens(db).await {
        tracing::warn!("Failed to purge expired in-game browser tokens: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_64_uppercase_digits_like_cpp_byte_array_to_hex_str() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xAB;
        bytes[31] = 0x0F;
        let hex = web_token_hex_like_cpp(&bytes);
        assert_eq!(hex.len(), WEB_TOKEN_HEX_LEN);
        assert!(hex.starts_with("AB00"));
        assert!(hex.ends_with("0F"));
        assert!(!hex.bytes().any(|byte| byte.is_ascii_lowercase()));
    }

    #[test]
    fn kind_round_trips_db_values() {
        assert_eq!(WebTokenKind::from_db(0), Some(WebTokenKind::WebCredentials));
        assert_eq!(WebTokenKind::from_db(1), Some(WebTokenKind::Sso));
        assert_eq!(WebTokenKind::from_db(2), None);
    }
}
