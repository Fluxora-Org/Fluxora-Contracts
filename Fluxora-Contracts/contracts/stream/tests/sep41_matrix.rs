#![cfg(test)]
extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient};
use soroban_sdk::{contractimpl, symbol_short, Address, Env, Map, Symbol};
use soroban_sdk::testutils::Address as _;

// Minimal mock token implementations for tests

#[derive(Default)]
pub struct RevokeToken;

#[contractimpl]
impl RevokeToken {
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let key = Symbol::new(&env, "bal");
        let mut balances: Map<Address, i128> = env.storage().instance().get(&key).unwrap_or_default();
        let from_bal = balances.get(&from).unwrap_or(0);
        assert!(from_bal >= amount, "insufficient balance");
        balances.set(from.clone(), from_bal - amount);
        let to_bal = balances.get(&to).unwrap_or(0);
        balances.set(to.clone(), to_bal + amount);
        env.storage().instance().set(&key, &balances);
    }

    pub fn transfer_from(env: Env, _invoker: Address, from: Address, to: Address, amount: i128) {
        let key_a = Symbol::new(&env, "allow");
        let mut allows: Map<Address, i128> = env.storage().instance().get(&key_a).unwrap_or_default();
        let allow = allows.get(&from).unwrap_or(0);
        if allow < amount {
            panic!("insufficient allowance");
        }
        allows.set(from.clone(), allow - amount);
        env.storage().instance().set(&key_a, &allows);
        Self::transfer(env, from, to, amount);
    }

    pub fn approve(env: Env, _spender: Address, owner: Address, amount: i128) {
        let key_a = Symbol::new(&env, "allow");
        let mut allows: Map<Address, i128> = env.storage().instance().get(&key_a).unwrap_or_default();
        allows.set(owner, amount);
        env.storage().instance().set(&key_a, &allows);
    }

    pub fn balance(env: Env, who: Address) -> i128 {
        let key = Symbol::new(&env, "bal");
        let balances: Map<Address, i128> = env.storage().instance().get(&key).unwrap_or_default();
        balances.get(&who).unwrap_or(0)
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let key = Symbol::new(&env, "bal");
        let mut balances: Map<Address, i128> = env.storage().instance().get(&key).unwrap_or_default();
        let cur = balances.get(&to).unwrap_or(0);
        balances.set(to, cur + amount);
        env.storage().instance().set(&key, &balances);
    }
}

#[derive(Default)]
pub struct ReturnFalseToken;

#[contractimpl]
impl ReturnFalseToken {
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) -> bool { false }
    pub fn transfer_from(_env: Env, _invoker: Address, _from: Address, _to: Address, _amount: i128) -> bool { false }
    pub fn approve(_env: Env, _spender: Address, _owner: Address, _amount: i128) -> bool { true }
    pub fn balance(_env: Env, _who: Address) -> i128 { 0 }
    pub fn mint(_env: Env, _to: Address, _amount: i128) {}
}

#[derive(Default)]
pub struct PanicToken;

#[contractimpl]
impl PanicToken {
    pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) { panic!("token transfer panicked"); }
    pub fn transfer_from(_env: Env, _invoker: Address, _from: Address, _to: Address, _amount: i128) { panic!("token transfer_from panicked"); }
    pub fn approve(_env: Env, _spender: Address, _owner: Address, _amount: i128) {}
    pub fn balance(_env: Env, _who: Address) -> i128 { 0 }
    pub fn mint(_env: Env, _to: Address, _amount: i128) {}
}

#[test]
fn sep41_matrix() {
    let env = Env::default();
    env.mock_all_auths();

    // Actors
    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    // Implementations
    let token_admin = Address::generate(&env);
    let normal_token_id = env.register_stellar_asset_contract_v2(token_admin).address();
    let revoke_token_id = env.register_contract(None, RevokeToken).address();
    let ret_false_id = env.register_contract(None, ReturnFalseToken).address();
    let panic_id = env.register_contract(None, PanicToken).address();

    let implementations = vec![
        ("normal", normal_token_id),
        ("revoke", revoke_token_id),
        ("ret_false", ret_false_id),
        ("panic", panic_id),
    ];

    for (name, token_addr) in implementations.into_iter() {
        // Re-register stream for clean state
        let stream_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &stream_id);
        client.init(&token_addr, &admin);

        // Try minting and approving where supported
        let _ = std::panic::catch_unwind(|| {
            env.invoke_contract(&token_addr, &Symbol::new(&env, "mint"), (sender.clone(), 1_000_i128).into_val(&env));
        });
        let _ = std::panic::catch_unwind(|| {
            env.invoke_contract(&token_addr, &Symbol::new(&env, "approve"), (stream_id.address(), sender.clone(), 1_000_i128).into_val(&env));
        });

        // create_stream
        let res = std::panic::catch_unwind(|| {
            client.create_stream(&sender, &recipient, &100_i128, &1_i128, &0u64, &0u64, &100u64, &0, &None);
        });

        if name == "normal" || name == "revoke" {
            assert!(res.is_ok(), "create_stream expected to succeed for {}", name);
        } else {
            assert!(res.is_err(), "create_stream expected to fail for {}", name);
        }

        // top_up_stream: for revoke simulate revoke
        let top_res = std::panic::catch_unwind(|| {
            if name == "revoke" {
                let _ = env.invoke_contract(&token_addr, &Symbol::new(&env, "approve"), (stream_id.address(), sender.clone(), 0_i128).into_val(&env));
            }
            client.top_up_stream(&0u64, &sender, &50_i128);
        });

        if name == "normal" {
            assert!(top_res.is_ok(), "top_up expected to succeed for normal");
        }

        // withdraw
        let wd_res = std::panic::catch_unwind(|| {
            env.ledger().set_timestamp(200);
            client.withdraw(&0u64);
        });

        if name == "panic" {
            assert!(wd_res.is_err(), "withdraw expected to panic for panic token");
        }

        // cancel_stream
        let cancel_res = std::panic::catch_unwind(|| {
            env.ledger().set_timestamp(300);
            client.cancel_stream(&0u64);
        });

        if name == "panic" {
            assert!(cancel_res.is_err(), "cancel expected to panic for panic token");
        }
    }
}
