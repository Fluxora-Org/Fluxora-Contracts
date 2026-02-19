#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

/// Global configuration for the Fluxora protocol.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Config {
    pub token: Address,
    pub admin: Address,
}

/// Namespace for all contract storage keys.
#[contracttype]
pub enum DataKey {
    Config,       // Instance storage for global settings.
    NextStreamId, // Instance storage for the auto-incrementing ID counter.
    Stream(u64),  // Persistent storage for individual stream data (O(1) lookup).
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
    Active = 0,
    Paused = 1,
    Completed = 2,
    Cancelled = 3,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Stream {
    pub stream_id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
    pub withdrawn_amount: i128,
    pub status: StreamStatus,
}

#[contract]
pub struct FluxoraStream;

#[contractimpl]
impl FluxoraStream {
    /// Initializes the stream contract by setting the admin and token addresses.
    pub fn init(env: Env, token: Address, admin: Address) {
        if env.storage().instance().has(&DataKey::Config) {
            panic!("Already initialized");
        }
        let config = Config { token, admin };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::NextStreamId, &1u64);
        
        // Ensure instance storage (Config/ID) doesn't expire quickly
        env.storage().instance().extend_ttl(17280, 120960);
    }

    /// Creates a new stream and persists it to the ledger.
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
    ) -> u64 {
        sender.require_auth();

        let stream_id: u64 = env.storage().instance().get(&DataKey::NextStreamId).unwrap_or(1);
        
        let stream = Stream {
            stream_id,
            sender,
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdrawn_amount: 0,
            status: StreamStatus::Active,
        };

        let key = DataKey::Stream(stream_id);
        env.storage().persistent().set(&key, &stream);
        
        // Requirement: Persistent storage for streams with TTL extension
        env.storage().persistent().extend_ttl(&key, 17280, 120960);
        
        // Update counter for next stream
        env.storage().instance().set(&DataKey::NextStreamId, &(stream_id + 1));
        
        stream_id
    }

    /// Fetches the global configuration.
    pub fn get_config(env: Env) -> Config {
        env.storage().instance().get(&DataKey::Config).expect("Not initialized")
    }

    /// Fetches the current state of a stream from persistent storage.
    pub fn get_stream_state(env: Env, stream_id: u64) -> Stream {
        env.storage()
            .persistent()
            .get(&DataKey::Stream(stream_id))
            .expect("Stream not found")
    }

    // Placeholders for future logic (Issue #2+)
    pub fn pause_stream(_env: Env, _stream_id: u64) {}
    pub fn resume_stream(_env: Env, _stream_id: u64) {}
    pub fn cancel_stream(_env: Env, _stream_id: u64) {}
    pub fn withdraw(_env: Env, _stream_id: u64) -> i128 { 0 }
    pub fn calculate_accrued(_env: Env, _stream_id: u64) -> i128 { 0 }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_initialization_and_config() {
        let env = Env::default();
        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        client.init(&token, &admin);
        
        let config = client.get_config();
        assert_eq!(config.admin, admin);
        assert_eq!(config.token, token);
    }

    #[test]
    #[should_panic(expected = "Already initialized")]
    fn test_cannot_init_twice() {
        let env = Env::default();
        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.init(&admin, &admin);
        client.init(&admin, &admin); // Should panic
    }

    #[test]
    fn test_stream_storage_and_increment() {
        let env = Env::default();
        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let token = Address::generate(&env);
        client.init(&token, &admin);

        env.mock_all_auths();
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Create first stream
        let id1 = client.create_stream(&sender, &recipient, &1000, &1, &100, &110, &200);
        assert_eq!(id1, 1);

        // Create second stream to check ID incrementing
        let id2 = client.create_stream(&sender, &recipient, &500, &1, &300, &310, &400);
        assert_eq!(id2, 2);

        // Verify storage for stream 1
        let stream1 = client.get_stream_state(&1);
        assert_eq!(stream1.deposit_amount, 1000);
        assert_eq!(stream1.sender, sender);

        // Verify storage for stream 2
        let stream2 = client.get_stream_state(&2);
        assert_eq!(stream2.deposit_amount, 500);
    }

    #[test]
    #[should_panic(expected = "Stream not found")]
    fn test_get_invalid_stream_panics() {
        let env = Env::default();
        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.get_stream_state(&99);
    }
}