//! Internal, non-ABI implementations backing the `#[contractimpl]` block
//! in `lib.rs` (issue #1520). Bodies are move-only extractions of the
//! original private associated fns, now `pub(crate)` free fns taking
//! `env: &Env`; exported signatures live on the lib.rs wrappers. The
//! split axis is lifecycle operation: exactly one public impl block
//! (thin wrappers) remains in lib.rs.

use soroban_sdk::symbol_short;
use soroban_sdk::{token, Address, Env, Map, Vec};
use crate::delegation;
use crate::events;
use crate::ops::validation;
use crate::*;

pub(crate) fn reserve_stream_ids(
    env: &Env,
    caller: Address,
    count: u32,
    expiry: Option<u64>,
) -> Result<soroban_sdk::Vec<u64>, ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            caller.require_auth();
    
            if count == 0 {
                return Err(ContractError::ReservationCountZero);
            }
            if count > MAX_ID_RESERVATION {
                return Err(ContractError::ReservationLimitExceeded);
            }
    
            if load_id_reservation(env, &caller).is_some() {
                return Err(ContractError::ReservationAlreadyActive);
            }
    
            let start_id = read_stream_count(env);
            set_stream_count(env, start_id + count as u64);
    
            let res = IdReservation {
                start_id,
                count,
                consumed: 0,
                expiry,
            };
            save_id_reservation(env, &caller, &res);
    
            let mut ids = soroban_sdk::Vec::new(env);
            for i in 0..count {
                ids.push_back(start_id + i as u64);
            }
            Ok(ids)
        
}

pub(crate) fn release_id_reservation(
    env: &Env,
    caller: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            caller.require_auth();
    
            let res = load_id_reservation(env, &caller).ok_or(ContractError::ReservationNotFound)?;
    
            crate::ops::id_reservation::release_reservation(env, &caller, &res);
            Ok(())
        
}

pub(crate) fn release_reservation(
    env: &Env,
    holder: &Address,
    res: &IdReservation,
) {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let unconsumed_start = res.start_id + res.consumed as u64;
            let reservation_end = res.start_id + res.count as u64;
            let current_count = read_stream_count(env);
    
            let mut reclaimed = 0u32;
    
            if reservation_end == current_count && unconsumed_start < reservation_end {
                set_stream_count(env, unconsumed_start);
                reclaimed = (reservation_end - unconsumed_start) as u32;
            }
    
            remove_id_reservation(env, holder);
    
            env.events().publish(
                (symbol_short!("res_rel"), holder.clone()),
                (res.start_id, res.count, res.consumed, reclaimed),
            );
        
}

pub(crate) fn reclaim_expired_id_reservation(
    env: &Env,
    holder: Address,
) -> Result<(), ContractError> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            let res = load_id_reservation(env, &holder).ok_or(ContractError::ReservationNotFound)?;
    
            let expiry = res.expiry.ok_or(ContractError::ReservationNotExpirable)?;
            if env.ledger().timestamp() < expiry {
                return Err(ContractError::ReservationStillActive);
            }
    
            crate::ops::id_reservation::release_reservation(env, &holder, &res);
            Ok(())
        
}

pub(crate) fn get_id_reservation(
    env: &Env,
    caller: Address,
) -> Option<IdReservation> {
    // Move-only extraction from `lib.rs` (issue #1520). Doc: see wrapper.
    
            load_id_reservation(env, &caller)
        
}

