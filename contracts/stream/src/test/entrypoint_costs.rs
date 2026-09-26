//! Deterministic, successful-call cost fixtures for the release WASM ABI.
//! Each printed value covers the last invocation only; setup is excluded.

use super::common::*;
use crate::op;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

fn record(h: &Harness, name: &str) {
    let instructions = h.env.cost_estimate().resources().instructions;
    assert!(instructions > 0, "{name} did not produce a cost estimate");
    std::println!("ENTRYPOINT_COST {name} {instructions}");
}

fn wasm_harness() -> Harness<'static> {
    let h = Harness::new();
    let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/wasm32v1-none/release/fluxora_stream.wasm");
    let wasm = std::fs::read(&wasm_path).expect("build the release stream WASM before measuring");
    h.env.register_at(&h.contract_id, wasm.as_slice(), ());
    h
}

fn fresh() -> (Harness<'static>, u64) {
    let h = wasm_harness();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    (h, id)
}

#[test]
#[ignore = "requires release WASM; run with script/validate_gas.py"]
fn entrypoint_cost_snapshot() {
    let h = wasm_harness();
    h.create_simple(1_000 * ONE, 100 * DAY);
    record(&h, "create_stream");

    let (h, id) = fresh();
    h.client.top_up(&id, &(100 * ONE));
    record(&h, "top_up");

    let (h, id) = fresh();
    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);
    record(&h, "withdraw");

    let (h, id) = fresh();
    h.advance(10 * DAY);
    h.client.batch_withdraw(&h.recipient, &h.ids(&[id]));
    record(&h, "batch_withdraw");

    let (h, id) = fresh();
    h.advance(10 * DAY);
    h.client.cancel(&id);
    record(&h, "cancel");

    let (h, id) = fresh();
    h.client.pause(&id);
    record(&h, "pause");

    let (h, id) = fresh();
    h.client.pause(&id);
    h.client.resume(&id);
    record(&h, "resume");

    let (h, id) = fresh();
    h.client.transfer_recipient(&id, &h.other);
    record(&h, "transfer_recipient");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &delegate, &op::WITHDRAW, &None);
    record(&h, "grant_delegate");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &delegate, &op::WITHDRAW, &None);
    h.client.revoke_delegate(&id, &h.recipient, &delegate);
    record(&h, "revoke_delegate");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &delegate, &op::WITHDRAW, &None);
    h.advance(10 * DAY);
    h.client.delegate_withdraw(&id, &delegate, &None);
    record(&h, "delegate_withdraw");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &delegate, &op::CANCEL, &None);
    h.advance(10 * DAY);
    h.client.delegate_cancel(&id, &delegate);
    record(&h, "delegate_cancel");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &delegate, &op::PAUSE, &None);
    h.client.delegate_pause(&id, &delegate);
    record(&h, "delegate_pause");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &delegate, &(op::PAUSE | op::RESUME), &None);
    h.client.delegate_pause(&id, &delegate);
    h.client.delegate_resume(&id, &delegate);
    record(&h, "delegate_resume");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.sender, &delegate, &op::TOP_UP, &None);
    h.client.delegate_top_up(&id, &delegate, &(100 * ONE));
    record(&h, "delegate_top_up");

    let (h, id) = fresh();
    let delegate = Address::generate(&h.env);
    h.client
        .grant_delegate(&id, &h.recipient, &delegate, &op::TRANSFER_RECIPIENT, &None);
    h.client
        .delegate_transfer_recipient(&id, &delegate, &h.other);
    record(&h, "delegate_transfer_recipient");

    let (h, id) = fresh();
    h.client.get_stream(&id);
    record(&h, "get_stream");

    let (h, id) = fresh();
    h.client.withdrawable_of(&id);
    record(&h, "withdrawable_of");

    let (h, id) = fresh();
    h.client.vested_of(&id);
    record(&h, "vested_of");

    let (h, id) = fresh();
    h.client.refundable_of(&id);
    record(&h, "refundable_of");

    let (h, _) = fresh();
    h.client.stream_count();
    record(&h, "stream_count");

    let (h, id) = fresh();
    h.client.stream_exists(&id);
    record(&h, "stream_exists");

    let (h, id) = fresh();
    h.client.extend_stream_ttl(&id);
    record(&h, "extend_stream_ttl");

    let (h, id) = fresh();
    h.client.batch_extend_ttl(&h.ids(&[id]));
    record(&h, "batch_extend_ttl");
}
