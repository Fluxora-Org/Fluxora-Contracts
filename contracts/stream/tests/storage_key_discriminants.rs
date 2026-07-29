//! Regression test for `DataKey` discriminant stability.
//!
//! Guards `docs/storage.md`'s discriminant table. The stream contract persists
//! state by the exact declaration-order discriminant of every variant in the
//! `DataKey` enum. Any silent reorder or mid-enum insertion would corrupt
//! every persistent entry on a live instance.
//!
//! Adding a new variant?
//! 1. Append it at the END of `DataKey` in `contracts/stream/src/lib.rs`.
//! 2. Add the matching `check_disc` assertion to this test.
//! 3. Update the discriminant table in `docs/storage.md`.

#![cfg(test)]

use fluxora_stream::DataKey;
use soroban_sdk::{Env, IntoVal};
use soroban_sdk::xdr::ScVal;

/// Asserts the exact `u32` representation of every variant listed in `docs/storage.md`.
///
/// In Soroban, `#[contracttype]` enums are serialized as `ScVec` where the 0th
/// element is the variant index (discriminant) as a `u32`.
#[test]
fn test_data_key_discriminants_are_stable() {
    let env = Env::default();

    let check_disc = |key: &DataKey, expected_name: &str, expected_disc: u32| {
        // IntoVal<Env, Val> converts DataKey to a Val
        let val = key.clone().into_val(&env);
        // Convert to ScVal to inspect the raw XDR representation
        let sc_val: ScVal = val.try_into_val(&env).expect("Failed to convert to ScVal");
        
        match sc_val {
            ScVal::Vec(Some(vec)) => {
                match &vec[0] {
                    ScVal::U32(d) => assert_eq!(
                        *d, expected_disc,
                        "DataKey::{} has discriminant {}, expected {}",
                        expected_name, d, expected_disc
                    ),
                    ScVal::Symbol(s) => {
                        // If newer Soroban SDK serializes as Symbol, we assert the symbol name
                        // but the index is what we care about. 
                        // We panic here because storage.md expects a 0-based discriminant.
                        panic!("Serialized as Symbol instead of U32: {:?}", s);
                    },
                    _ => panic!("Unexpected 0th element in ScVec for DataKey::{}", expected_name),
                }
            },
            _ => panic!("DataKey::{} not serialized as ScVec", expected_name),
        }
    };

    let dummy_addr = soroban_sdk::Address::generate(&env);

    check_disc(&DataKey::Config, "Config", 0);
    check_disc(&DataKey::NextStreamId, "NextStreamId", 1);
    check_disc(&DataKey::Stream(0), "Stream", 2);
    check_disc(&DataKey::RecipientStreams(dummy_addr.clone()), "RecipientStreams", 3);
    check_disc(&DataKey::GlobalEmergencyPaused, "GlobalEmergencyPaused", 4);
    check_disc(&DataKey::CreationPaused, "CreationPaused", 5);
    check_disc(&DataKey::GlobalPauseReason, "GlobalPauseReason", 6);
    check_disc(&DataKey::GlobalPauseTimestamp, "GlobalPauseTimestamp", 7);
    check_disc(&DataKey::GlobalPauseAdmin, "GlobalPauseAdmin", 8);
    check_disc(&DataKey::AutoClaimDestination(0), "AutoClaimDestination", 9);
    check_disc(&DataKey::NextTemplateId, "NextTemplateId", 10);
    check_disc(&DataKey::ActiveTemplateCount, "ActiveTemplateCount", 11);
    check_disc(&DataKey::StreamTemplate(0), "StreamTemplate", 12);
    check_disc(&DataKey::OwnerTemplateIds(dummy_addr.clone()), "OwnerTemplateIds", 13);
    check_disc(&DataKey::TotalLiabilities, "TotalLiabilities", 14);
    check_disc(&DataKey::WithdrawNonce(dummy_addr.clone()), "WithdrawNonce", 15);
    check_disc(&DataKey::PauseState, "PauseState", 16);
    check_disc(&DataKey::ReentrancyLock, "ReentrancyLock", 17);
    check_disc(&DataKey::RecipientStreamPage(dummy_addr.clone(), 0), "RecipientStreamPage", 18);
    check_disc(&DataKey::RecipientStreamPageCount(dummy_addr.clone()), "RecipientStreamPageCount", 19);
    check_disc(&DataKey::PendingRecipientUpdate(0), "PendingRecipientUpdate", 20);
    check_disc(&DataKey::IdReservation(dummy_addr.clone()), "IdReservation", 21);
    check_disc(&DataKey::MaxRatePerSecond, "MaxRatePerSecond", 22);
    check_disc(&DataKey::DelegatedWithdrawNonce(dummy_addr.clone()), "DelegatedWithdrawNonce", 23);
    check_disc(&DataKey::LastPauseRecord(fluxora_stream::PauseKind::Stream), "LastPauseRecord", 24);
    check_disc(&DataKey::RotationHistory(0), "RotationHistory", 25);
    check_disc(&DataKey::LastAccrualLedgerTimestamp, "LastAccrualLedgerTimestamp", 26);
    check_disc(&DataKey::PausedStreamCount, "PausedStreamCount", 27);
    check_disc(&DataKey::TotalKeeperFeesPaid, "TotalKeeperFeesPaid", 28);
}
