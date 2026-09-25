//! Delete-method selection and undelete decision chain against a scripted port.

use std::sync::Mutex;

use wow_persistence::{DeletedCharacterInfoLikeCpp, PersistenceFutureLikeCpp};

use super::*;

#[derive(Default)]
struct FakePort {
    last_undelete: Option<u32>,
    deleted: Option<DeletedCharacterInfoLikeCpp>,
    name_taken: bool,
    character_count: u64,
    restore_fails: bool,
    calls: Mutex<Vec<String>>,
}

impl FakePort {
    fn log(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl CharacterUndeletePersistencePortLikeCpp for FakePort {
    fn load_delete_candidate_like_cpp(
        &self,
        _guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterDeleteCandidateLikeCpp>> {
        Box::pin(async { LoadOutcome::NotFound })
    }

    fn unlink_owned_character_like_cpp(
        &self,
        guid: u64,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        self.log(format!("unlink {guid} {account_id}"));
        Box::pin(async { MutationOutcome::Applied })
    }

    fn load_last_character_undelete_like_cpp(
        &self,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u32>> {
        self.log(format!("last {battlenet_account_id}"));
        let outcome = match self.last_undelete {
            Some(last) => LoadOutcome::Loaded(last),
            None => LoadOutcome::NotFound,
        };
        Box::pin(async move { outcome })
    }

    fn load_deleted_character_like_cpp(
        &self,
        guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<DeletedCharacterInfoLikeCpp>> {
        self.log(format!("deleted {guid}"));
        let outcome = match &self.deleted {
            Some(info) => LoadOutcome::Loaded(info.clone()),
            None => LoadOutcome::NotFound,
        };
        Box::pin(async move { outcome })
    }

    fn find_character_name_like_cpp(
        &self,
        name: String,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<()>> {
        self.log(format!("name {name}"));
        let taken = self.name_taken;
        Box::pin(async move {
            if taken {
                LoadOutcome::Loaded(())
            } else {
                LoadOutcome::NotFound
            }
        })
    }

    fn load_account_character_count_like_cpp(
        &self,
        account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u64>> {
        self.log(format!("count {account_id}"));
        let count = self.character_count;
        Box::pin(async move { LoadOutcome::Loaded(count) })
    }

    fn restore_deleted_character_like_cpp(
        &self,
        guid: u64,
        name: String,
        account_id: u32,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        self.log(format!(
            "restore {guid} {name} {account_id} {battlenet_account_id}"
        ));
        let fails = self.restore_fails;
        Box::pin(async move {
            if fails {
                MutationOutcome::Failed {
                    reason: "boom".into(),
                }
            } else {
                MutationOutcome::Applied
            }
        })
    }

    fn reset_undelete_cooldown_like_cpp(
        &self,
        battlenet_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        self.log(format!("reset {battlenet_account_id}"));
        Box::pin(async { MutationOutcome::Applied })
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime")
        .block_on(future)
}

const NOW: u32 = 1_800_000_000;
const COOLDOWN: u32 = 2_592_000;

fn request() -> UndeleteRequestLikeCpp {
    UndeleteRequestLikeCpp {
        guid_low: 42,
        account_id: 7,
        battlenet_account_id: 3,
        enabled: true,
        max_cooldown: COOLDOWN,
        characters_per_realm: 10,
        now: NOW,
    }
}

fn restorable() -> FakePort {
    FakePort {
        last_undelete: Some(0),
        deleted: Some(DeletedCharacterInfoLikeCpp {
            name: "Lazarus".into(),
            account_id: 7,
        }),
        character_count: 3,
        ..FakePort::default()
    }
}

fn run(port: &FakePort, request: UndeleteRequestLikeCpp) -> u32 {
    block_on(undelete_character_like_cpp(Some(port), request))
}

#[test]
fn delete_method_follows_config_and_class_minimum_levels_like_cpp() {
    let config = CharacterDeletionConfigLikeCpp {
        delete_method: CHAR_DELETE_UNLINK_LIKE_CPP,
        min_level: 10,
        death_knight_min_level: 60,
        demon_hunter_min_level: 100,
        ..CharacterDeletionConfigLikeCpp::default()
    };
    let candidate = |class, level| Some(CharacterDeleteCandidateLikeCpp { class, level });
    assert_eq!(
        delete_method_like_cpp(&config, None),
        CHAR_DELETE_UNLINK_LIKE_CPP
    );
    assert_eq!(
        delete_method_like_cpp(&config, candidate(1, 9)),
        CHAR_DELETE_REMOVE_LIKE_CPP
    );
    assert_eq!(
        delete_method_like_cpp(&config, candidate(1, 10)),
        CHAR_DELETE_UNLINK_LIKE_CPP
    );
    assert_eq!(
        delete_method_like_cpp(&config, candidate(6, 59)),
        CHAR_DELETE_REMOVE_LIKE_CPP
    );
    assert_eq!(
        delete_method_like_cpp(&config, candidate(6, 60)),
        CHAR_DELETE_UNLINK_LIKE_CPP
    );
    assert_eq!(
        delete_method_like_cpp(&CharacterDeletionConfigLikeCpp::default(), candidate(1, 80)),
        CHAR_DELETE_REMOVE_LIKE_CPP
    );
}

#[test]
fn cooldown_arithmetic_matches_both_cpp_callbacks() {
    assert_eq!(undelete_cooldown_remaining_like_cpp(0, COOLDOWN, NOW), 0);
    assert_eq!(
        undelete_cooldown_remaining_like_cpp(NOW - 100, COOLDOWN, NOW),
        COOLDOWN - 100
    );
    assert_eq!(
        undelete_cooldown_remaining_like_cpp(NOW - COOLDOWN, COOLDOWN, NOW),
        0
    );
    assert!(!undelete_on_cooldown_like_cpp(0, u32::MAX, NOW));
    assert!(undelete_on_cooldown_like_cpp(NOW - 1, COOLDOWN, NOW));
    assert!(!undelete_on_cooldown_like_cpp(
        NOW - COOLDOWN,
        COOLDOWN,
        NOW
    ));
}

#[test]
fn cooldown_status_reports_remaining_time_or_zero() {
    let port = FakePort {
        last_undelete: Some(NOW - 60),
        ..FakePort::default()
    };
    let status = block_on(undelete_cooldown_status_like_cpp(
        Some(&port),
        3,
        COOLDOWN,
        NOW,
    ));
    assert!(status.on_cooldown);
    assert_eq!(status.max_cooldown, COOLDOWN);
    assert_eq!(status.current_cooldown, COOLDOWN - 60);

    let fresh = block_on(undelete_cooldown_status_like_cpp(
        Some(&FakePort::default()),
        3,
        COOLDOWN,
        NOW,
    ));
    assert!(!fresh.on_cooldown);
    assert_eq!(fresh.current_cooldown, 0);
    assert_eq!(fresh.max_cooldown, COOLDOWN);
}

#[test]
fn disabled_undelete_touches_no_database() {
    let port = restorable();
    let result = run(
        &port,
        UndeleteRequestLikeCpp {
            enabled: false,
            ..request()
        },
    );
    assert_eq!(result, undelete_result::ERROR_DISABLED);
    assert!(port.calls().is_empty());
}

#[test]
fn undelete_on_cooldown_is_refused_before_reading_the_character() {
    let port = FakePort {
        last_undelete: Some(NOW - 10),
        ..restorable()
    };
    assert_eq!(run(&port, request()), undelete_result::ERROR_COOLDOWN);
    assert_eq!(port.calls(), vec!["last 3"]);
}

#[test]
fn missing_deleted_character_answers_char_create() {
    let port = FakePort {
        deleted: None,
        ..restorable()
    };
    assert_eq!(run(&port, request()), undelete_result::ERROR_CHAR_CREATE);
}

#[test]
fn deleted_character_of_another_account_answers_unknown() {
    let port = FakePort {
        deleted: Some(DeletedCharacterInfoLikeCpp {
            name: "Lazarus".into(),
            account_id: 8,
        }),
        ..restorable()
    };
    assert_eq!(run(&port, request()), undelete_result::ERROR_UNKNOWN);
    assert!(!port.calls().iter().any(|call| call.starts_with("restore")));
}

#[test]
fn taken_name_answers_name_taken_by_this_account() {
    let port = FakePort {
        name_taken: true,
        ..restorable()
    };
    assert_eq!(
        run(&port, request()),
        undelete_result::ERROR_NAME_TAKEN_BY_THIS_ACCOUNT
    );
}

#[test]
fn full_account_answers_char_create() {
    let port = FakePort {
        character_count: 10,
        ..restorable()
    };
    assert_eq!(run(&port, request()), undelete_result::ERROR_CHAR_CREATE);
    assert!(!port.calls().iter().any(|call| call.starts_with("restore")));
}

#[test]
fn successful_undelete_runs_the_cpp_chain_in_order() {
    let port = restorable();
    assert_eq!(run(&port, request()), undelete_result::OK);
    assert_eq!(
        port.calls(),
        vec![
            "last 3",
            "deleted 42",
            "name Lazarus",
            "count 7",
            "restore 42 Lazarus 7 3",
        ]
    );
}

#[test]
fn failed_restore_is_not_reported_as_ok() {
    let port = FakePort {
        restore_fails: true,
        ..restorable()
    };
    assert_eq!(run(&port, request()), undelete_result::ERROR_UNKNOWN);
}

#[test]
fn missing_account_row_is_not_a_cooldown() {
    let port = FakePort {
        last_undelete: None,
        ..restorable()
    };
    assert_eq!(run(&port, request()), undelete_result::OK);
}
