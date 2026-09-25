//! In-game browser web tokens stored in `auth.battlenet_account_web_token`.
//!
//! Port of `LegionCore` `Battlenet::AuthenticationService::IssueToken`
//! (`src/server/game/Services/WorldserverService.cpp`): a token is 32 random bytes
//! rendered as 64 uppercase hex characters, inserted with
//! `LOGIN_INS_BNET_WEB_TOKEN` (`expires = NOW() + Browser.TokenLifetime`) and followed
//! by an opportunistic `LOGIN_DEL_BNET_WEB_TOKENS_EXPIRED`. The web validates the
//! token against the same table.
//!
//! The token row type and its INSERT/DELETE helpers live in
//! `wow_database::web_token`, shared with world-server (which issues the kind-1
//! checkout SSO token for `CMSG_BATTLE_PAY_OPEN_CHECKOUT`). This module keeps the
//! random generation and the web-side validation helpers.
#![allow(dead_code)]

use wow_database::{DatabaseError, LoginDatabase, LoginStatements};

// The token row (kind, issue metadata, INSERT/DELETE SQL) is shared with
// world-server, which issues the kind-1 checkout SSO token; see
// `wow_database::web_token`.
#[cfg(test)]
use wow_database::web_token::purge_expired_web_tokens;
pub use wow_database::web_token::{
    WEB_TOKEN_HEX_LEN, WebTokenIssue, WebTokenKind, insert_web_token,
    purge_expired_web_tokens_best_effort,
};

/// Row returned by [`validate_web_token`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebTokenInfo {
    pub battlenet_account: u32,
    pub account: u32,
    pub realm: u32,
    pub character_guid: u64,
    pub program: u32,
    pub kind: Option<WebTokenKind>,
    /// Unix expiry timestamp.
    pub expires: u64,
}

/// Why a presented token was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebTokenRejection {
    /// Not 64 hex characters (never hits the database).
    MalformedToken,
    /// No row for the token.
    UnknownToken,
    /// Row exists but `expires < now`.
    Expired,
    /// Row belongs to another Battle.net account.
    AccountMismatch,
}

/// 32 random bytes as 64 uppercase hex characters, like C++ `ByteArrayToHexStr`.
pub fn make_web_token_like_cpp() -> String {
    wow_database::web_token::web_token_hex_like_cpp(&random_token_bytes())
}

fn random_token_bytes() -> [u8; 32] {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes);
    bytes
}

/// A token is well formed when it is exactly 64 hex digits (either case).
pub fn is_well_formed_web_token(token: &str) -> bool {
    token.len() == WEB_TOKEN_HEX_LEN && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Mirrors `LOGIN_DEL_BNET_WEB_TOKENS_EXPIRED` (`expires < NOW()`): a token is valid
/// through the second it expires.
pub fn web_token_is_expired_like_cpp(expires: u64, now: u64) -> bool {
    expires < now
}

/// Pure acceptance check shared by [`validate_web_token`] and its tests.
pub fn check_web_token_row(
    row: Option<&WebTokenInfo>,
    expected_battlenet_account: u32,
    now: u64,
) -> Result<(), WebTokenRejection> {
    let Some(row) = row else {
        return Err(WebTokenRejection::UnknownToken);
    };
    if web_token_is_expired_like_cpp(row.expires, now) {
        return Err(WebTokenRejection::Expired);
    }
    if row.battlenet_account != expected_battlenet_account {
        return Err(WebTokenRejection::AccountMismatch);
    }
    Ok(())
}

/// `LegionCore` `IssueToken`: generate a fresh token, store it and purge expired rows.
pub async fn issue_web_token(
    db: &LoginDatabase,
    issue: &WebTokenIssue,
) -> Result<String, DatabaseError> {
    wow_database::web_token::issue_web_token_from_bytes(db, issue, &random_token_bytes()).await
}

/// Load a token row; `None` when it does not exist.
pub async fn load_web_token(
    db: &LoginDatabase,
    token: &str,
) -> Result<Option<(WebTokenInfo, u64)>, DatabaseError> {
    let mut stmt = db.prepare(LoginStatements::SEL_BNET_WEB_TOKEN);
    stmt.set_string(0, token);
    let result = db.query(&stmt).await?;
    if result.is_empty() {
        return Ok(None);
    }

    let info = WebTokenInfo {
        battlenet_account: result.try_read::<u32>(0).unwrap_or(0),
        account: result.try_read::<u32>(1).unwrap_or(0),
        realm: result.try_read::<u32>(2).unwrap_or(0),
        character_guid: result.try_read::<u64>(3).unwrap_or(0),
        program: result.try_read::<u32>(4).unwrap_or(0),
        kind: result.try_read::<u8>(5).and_then(WebTokenKind::from_db),
        expires: result.try_read::<i64>(6).unwrap_or(0).max(0) as u64,
    };
    let db_now = result.try_read::<i64>(7).unwrap_or(0).max(0) as u64;
    Ok(Some((info, db_now)))
}

/// Validate a presented token: well formed, stored, not expired (against the database
/// clock, the same one `expires` was computed with) and owned by `battlenet_account`.
pub async fn validate_web_token(
    db: &LoginDatabase,
    token: &str,
    battlenet_account: u32,
) -> Result<Result<WebTokenInfo, WebTokenRejection>, DatabaseError> {
    if !is_well_formed_web_token(token) {
        return Ok(Err(WebTokenRejection::MalformedToken));
    }

    let Some((info, db_now)) = load_web_token(db, token).await? else {
        return Ok(Err(WebTokenRejection::UnknownToken));
    };

    Ok(check_web_token_row(Some(&info), battlenet_account, db_now).map(|()| info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(battlenet_account: u32, expires: u64) -> WebTokenInfo {
        WebTokenInfo {
            battlenet_account,
            account: 7,
            realm: 1,
            character_guid: 0,
            program: 0x0057_6F57,
            kind: Some(WebTokenKind::WebCredentials),
            expires,
        }
    }

    #[test]
    fn make_web_token_is_64_uppercase_hex_like_cpp_byte_array_to_hex_str() {
        let token = make_web_token_like_cpp();

        assert_eq!(token.len(), WEB_TOKEN_HEX_LEN);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!token.bytes().any(|byte| matches!(byte, b'a'..=b'f')));
        assert!(is_well_formed_web_token(&token));
    }

    #[test]
    fn make_web_token_is_random_per_call() {
        assert_ne!(make_web_token_like_cpp(), make_web_token_like_cpp());
    }

    #[test]
    fn well_formed_web_token_requires_exactly_64_hex_digits() {
        assert!(is_well_formed_web_token(&"A".repeat(64)));
        assert!(is_well_formed_web_token(&"f".repeat(64)));
        assert!(!is_well_formed_web_token(&"A".repeat(63)));
        assert!(!is_well_formed_web_token(&"A".repeat(65)));
        assert!(!is_well_formed_web_token(&format!("{}G", "A".repeat(63))));
        assert!(!is_well_formed_web_token(
            "TC-0123456789ABCDEF0123456789ABCDEF01234567"
        ));
        assert!(!is_well_formed_web_token(""));
    }

    #[test]
    fn web_token_expiry_matches_cpp_delete_predicate() {
        // DELETE ... WHERE expires < NOW(): valid through the expiry second.
        assert!(web_token_is_expired_like_cpp(99, 100));
        assert!(!web_token_is_expired_like_cpp(100, 100));
        assert!(!web_token_is_expired_like_cpp(101, 100));
    }

    #[test]
    fn web_token_kind_round_trips_db_values() {
        assert_eq!(WebTokenKind::WebCredentials as u8, 0);
        assert_eq!(WebTokenKind::Sso as u8, 1);
        assert_eq!(WebTokenKind::from_db(0), Some(WebTokenKind::WebCredentials));
        assert_eq!(WebTokenKind::from_db(1), Some(WebTokenKind::Sso));
        assert_eq!(WebTokenKind::from_db(2), None);
    }

    #[test]
    fn check_web_token_row_accepts_live_row_for_owner() {
        assert_eq!(check_web_token_row(Some(&row(5, 1_000)), 5, 1_000), Ok(()));
        assert_eq!(check_web_token_row(Some(&row(5, 1_000)), 5, 999), Ok(()));
    }

    #[test]
    fn check_web_token_row_rejects_missing_expired_and_foreign_rows() {
        assert_eq!(
            check_web_token_row(None, 5, 1_000),
            Err(WebTokenRejection::UnknownToken)
        );
        assert_eq!(
            check_web_token_row(Some(&row(5, 999)), 5, 1_000),
            Err(WebTokenRejection::Expired)
        );
        assert_eq!(
            check_web_token_row(Some(&row(6, 1_000)), 5, 1_000),
            Err(WebTokenRejection::AccountMismatch)
        );
        // Expiry is checked before ownership, like a purge would have removed the row.
        assert_eq!(
            check_web_token_row(Some(&row(6, 1)), 5, 1_000),
            Err(WebTokenRejection::Expired)
        );
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Round trip against a live `battlenet_account_web_token` table (scratch or real auth).
    #[tokio::test]
    #[ignore = "requires a live auth database with battlenet_account_web_token; set RUSTYCORE_DB_IT_USER and optional HOST/PORT/PASS/AUTH_DB"]
    async fn live_web_token_insert_validate_and_purge_round_trip() {
        let Some(user) = std::env::var("RUSTYCORE_DB_IT_USER").ok() else {
            eprintln!("skipping: RUSTYCORE_DB_IT_USER is not set");
            return;
        };
        let host = std::env::var("RUSTYCORE_DB_IT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = std::env::var("RUSTYCORE_DB_IT_PORT").unwrap_or_else(|_| "3306".into());
        let password = std::env::var("RUSTYCORE_DB_IT_PASS").unwrap_or_default();
        let database = std::env::var("RUSTYCORE_DB_IT_AUTH_DB").unwrap_or_else(|_| "auth".into());
        let url = format!("mysql://{user}:{password}@{host}:{port}/{database}?ssl-mode=DISABLED");
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect to live auth database");
        let db = LoginDatabase::from_pool(pool);

        let issue = WebTokenIssue {
            battlenet_account: 4_000_000_001,
            account: 4_000_000_002,
            realm: 1,
            character_guid: 0,
            program: 0x0057_6F57,
            kind: WebTokenKind::WebCredentials,
            ip: "203.0.113.10".to_string(),
            lifetime_secs: 60,
        };
        let token = issue_web_token(&db, &issue).await.expect("issue token");
        assert!(is_well_formed_web_token(&token));

        let accepted = validate_web_token(&db, &token, issue.battlenet_account)
            .await
            .expect("query token");
        let info = accepted.expect("fresh token is valid for its owner");
        assert_eq!(info.battlenet_account, issue.battlenet_account);
        assert_eq!(info.account, issue.account);
        assert_eq!(info.program, issue.program);
        assert_eq!(info.kind, Some(WebTokenKind::WebCredentials));

        assert_eq!(
            validate_web_token(&db, &token, issue.battlenet_account + 1)
                .await
                .expect("query token"),
            Err(WebTokenRejection::AccountMismatch)
        );
        assert_eq!(
            validate_web_token(&db, "not-a-token", issue.battlenet_account)
                .await
                .expect("query token"),
            Err(WebTokenRejection::MalformedToken)
        );
        assert_eq!(
            validate_web_token(&db, &"0".repeat(64), issue.battlenet_account)
                .await
                .expect("query token"),
            Err(WebTokenRejection::UnknownToken)
        );

        // Re-persisting the same credential is an upsert (bnetserver stores the login ticket).
        insert_web_token(&db, &token, &issue)
            .await
            .expect("upsert keeps the primary key");

        // Force expiry and check both the validator and the purge agree.
        let expired = WebTokenIssue {
            lifetime_secs: 0,
            ..issue.clone()
        };
        insert_web_token(&db, &token, &expired)
            .await
            .expect("upsert");
        let mut wait = 0;
        while wait < 30 {
            let outcome = validate_web_token(&db, &token, issue.battlenet_account)
                .await
                .expect("query token");
            if outcome == Err(WebTokenRejection::Expired) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            wait += 1;
        }
        // expires < NOW() needs the next DB second to tick over.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert_eq!(
            validate_web_token(&db, &token, issue.battlenet_account)
                .await
                .expect("query token"),
            Err(WebTokenRejection::Expired)
        );
        assert!(purge_expired_web_tokens(&db).await.expect("purge") >= 1);
        assert_eq!(
            validate_web_token(&db, &token, issue.battlenet_account)
                .await
                .expect("query token"),
            Err(WebTokenRejection::UnknownToken)
        );
    }
}
