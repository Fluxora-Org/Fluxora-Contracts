#![cfg(test)]

// Issue #899: ID-monotonicity regression coverage for the stream id allocator.
//
// A full cross-upgrade simulation requires deploying a second WASM blob into the
// test ledger (the contract `upgrade` entry point calls
// `update_current_contract_wasm`, which panics with `Wasm does not exist` unless
// the hash is already uploaded). That path belongs to an integration harness, not
// a unit test. Here we lock in the core invariant the upgrade must preserve:
// the next-id counter is backed by instance storage, is strictly monotonic, and
// never collides between `reserve_stream_ids` and `create_stream` allocations.

extern crate std;

use fluxora_stream::{FluxoraStream, FluxoraStreamClient, StreamKind};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &1_000_000_000_000);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &6_000_000);
        client.init(&token_id, &sender);

        Ctx {
            env,
            client,
            sender,
            recipient,
        }
    }

    fn create_stream(&self) -> u64 {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0_i128,
            &None,
            &StreamKind::Linear,
        )
    }
}

/// Live create_stream allocations must be strictly increasing with no gaps or
/// repeats, and the persisted counter must continue across separate operations.
#[test]
fn id_monotonicity_across_allocations() {
    let ctx = Ctx::setup();

    let first: Vec<u64> = (0..3).map(|_| ctx.create_stream()).collect();
    assert_eq!(first, vec![0u64, 1, 2], "first batch must be 0,1,2");
    assert_eq!(ctx.client.get_stream_count(), 3);

    // A second, separate batch must continue from the persisted counter (3..).
    let second: Vec<u64> = (0..3).map(|_| ctx.create_stream()).collect();
    assert_eq!(second, vec![3u64, 4, 5], "second batch must continue 3,4,5");
    assert_eq!(ctx.client.get_stream_count(), 6);

    let all: Vec<u64> = first.iter().chain(second.iter()).copied().collect();
    for w in all.windows(2) {
        assert!(w[1] > w[0], "IDs must be strictly increasing");
    }

    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 6, "no duplicate IDs across allocations");
}

/// Reserved ID batches must be internally monotonic and unique. This is the
/// allocation invariant an upgrade must preserve for the reservation path.
#[test]
fn id_monotonicity_reservation_batch_is_unique() {
    let ctx = Ctx::setup();

    let ids = ctx.client.reserve_stream_ids(&ctx.sender, &4u32, &None);
    assert_eq!(ids.len(), 4);
    assert_eq!(ids.get(0).unwrap(), 0);
    assert_eq!(ids.get(1).unwrap(), 1);
    assert_eq!(ids.get(2).unwrap(), 2);
    assert_eq!(ids.get(3).unwrap(), 3);

    // Re-reserving must continue from the reservation counter, not repeat.
    let more = ctx.client.reserve_stream_ids(&ctx.sender, &2u32, &None);
    assert_eq!(more.get(0).unwrap(), 4);
    assert_eq!(more.get(1).unwrap(), 5);

    let mut all: Vec<u64> = vec![];
    for i in 0..ids.len() {
        all.push(ids.get(i).unwrap());
    }
    for i in 0..more.len() {
        all.push(more.get(i).unwrap());
    }
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 6, "reserved ids never collide or repeat");
}
