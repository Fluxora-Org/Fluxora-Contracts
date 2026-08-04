#![cfg(test)]
extern crate std;

use fluxora_stream::{
    ContractError, CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env,
};

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    sender: Address,
    recipient: Address,
    admin: Address,
}

mod mock_token {
    use soroban_sdk::{contract, contractimpl, Address, Env};
    #[contract]
    pub struct MockToken;
    #[contractimpl]
    impl MockToken {
        pub fn init(env: Env, _token: Address, _admin: Address) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn mint(env: Env, _to: Address, _amount: i128) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn approve(
            env: Env,
            _from: Address,
            _spender: Address,
            _amount: i128,
            _expiration_ledger: u32,
        ) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn transfer(env: Env, _from: Address, _to: Address, _amount: i128) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn transfer_from(
            env: Env,
            _spender: Address,
            _from: Address,
            _to: Address,
            _amount: i128,
        ) {
            env.storage().instance().extend_ttl(100_000, 100_000);
        }
        pub fn balance(env: Env, _id: Address) -> i128 {
            env.storage().instance().extend_ttl(100_000, 100_000);
            1_000_000_000
        }
    }
}

impl TestContext {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set(LedgerInfo {
            timestamp: 0,
            protocol_version: 20,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 16,
            max_entry_ttl: 6_312_000,
        });

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token_id = env.register_contract(None, mock_token::MockToken);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            sender,
            recipient,
            admin,
        }
    }

    fn client(&self) -> &FluxoraStreamClient {
        &self.client
    }

    fn create_stream(&self, deposit: i128, rate: i128) -> u64 {
        let params = CreateStreamParams {
            recipient: self.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: rate,
            start_time: self.env.ledger().timestamp(),
            cliff_time: self.env.ledger().timestamp() + 10,
            end_time: self.env.ledger().timestamp() + 100,
            withdraw_dust_threshold: Some(0),
            memo: None,
            kind: StreamKind::Linear,
            metadata: None,
            irrevocable: None,
            witness: None,
        };
        self.client.create_stream(&self.sender, &params)
    }
}

#[test]
fn test_withdraw_with_none_expected_amount_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(10000, 100);

    // Advance time to allow some withdrawal
    ctx.env.ledger().set(LedgerInfo {
        timestamp: 50,
        ..ctx.env.ledger().get()
    });

    // Withdraw with None (no min_expected_amount specified)
    let withdrawn = ctx.client().withdraw(&stream_id, &None);

    assert_eq!(withdrawn, 5000); // 50 seconds * 100 rate = 5000
}

#[test]
fn test_withdraw_with_min_expected_amount_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(10000, 100);

    // Advance time to allow some withdrawal
    ctx.env.ledger().set(LedgerInfo {
        timestamp: 50,
        ..ctx.env.ledger().get()
    });

    // Withdrawable amount is 5000. Expected min is 4000. This should succeed.
    let withdrawn = ctx.client().withdraw(&stream_id, &Some(4000));

    assert_eq!(withdrawn, 5000);
}

#[test]
fn test_withdraw_with_exact_min_expected_amount_succeeds() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(10000, 100);

    // Advance time to allow some withdrawal
    ctx.env.ledger().set(LedgerInfo {
        timestamp: 50,
        ..ctx.env.ledger().get()
    });

    // Withdrawable amount is 5000. Expected min is exactly 5000. This should succeed.
    let withdrawn = ctx.client().withdraw(&stream_id, &Some(5000));

    assert_eq!(withdrawn, 5000);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_withdraw_fails_when_below_min_expected_amount() {
    let ctx = TestContext::setup();
    let stream_id = ctx.create_stream(10000, 100);

    // Advance time to allow some withdrawal
    ctx.env.ledger().set(LedgerInfo {
        timestamp: 50,
        ..ctx.env.ledger().get()
    });

    // Withdrawable amount is 5000. Expected min is 6000. This should panic with BelowMinimumAmount.
    ctx.client().withdraw(&stream_id, &Some(6000));
}
