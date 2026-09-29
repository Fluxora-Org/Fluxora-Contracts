//! Deterministic, successful-call cost fixtures for the release WASM ABI.
//! Each printed value covers the last invocation only; setup is excluded.

use super::common::*;
use crate::{op, BatchCreateRequest};
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

/// Issue #1815 — the same setup as [`fresh`], but on a non-linear release
/// curve, so the curve-dispatching accrual path is measured rather than the
/// linear fast path alone.
fn fresh_curved(curve: crate::ReleaseCurve) -> (Harness<'static>, u64) {
    let h = wasm_harness();
    let start = h.now();
    let id = h.client.create_stream_with_curve(
        &h.sender,
        &h.recipient,
        &h.token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &curve,
    );
    (h, id)
}

#[test]
#[ignore = "requires release WASM; run with script/validate_gas.py"]
fn entrypoint_cost_snapshot() {
    let h = wasm_harness();
    h.create_simple(1_000 * ONE, 100 * DAY);
    record(&h, "create_stream");

    // Issue #1815 — the curve-carrying creation path.
    let h = wasm_harness();
    let start = h.now();
    h.client.create_stream_with_curve(
        &h.sender,
        &h.recipient,
        &h.token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &crate::ReleaseCurve::FrontLoaded,
    );
    record(&h, "create_stream_with_curve");
    let h = wasm_harness();
    let start = h.now();
    let mut requests = soroban_sdk::Vec::new(&h.env);
    for _ in 0..1 {
        requests.push_back(BatchCreateRequest {
            recipient: Address::generate(&h.env),
            token: h.token.clone(),
            deposit: 1_000 * ONE,
            start_time: start,
            end_time: start + 100 * DAY,
            cliff_time: start,
            cancellable: true,
            pausable: true,
            transferable: true,
            paused_at: None,
            paused_total: 0,
            status: crate::StreamStatus::Active,
        });
    }
    h.client.batch_create(&h.sender, &requests);
    record(&h, "batch_create");
    // Same call through the mode-taking entry point, so the baseline records the
    // extra argument decode and the stored enum rather than assuming it is free.
    let h = wasm_harness();
    h.create_with_cliff_mode(
        1_000 * ONE,
        h.env.ledger().timestamp(),
        h.env.ledger().timestamp() + 100 * DAY,
        h.env.ledger().timestamp() + 10 * DAY,
        crate::CliffMode::WallClock,
        true,
        true,
        true,
    );
    record(&h, "create_stream_with_cliff_mode");

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
    h.advance(10 * DAY);
    h.client.batch_cancel(&h.sender, &h.ids(&[id]));
    record(&h, "batch_cancel");

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

    // Issue #1815 — `vested` now dispatches on the stream's release curve, so
    // the pinned figure for this entry point is taken on a non-linear stream:
    // that is the curve-dispatching path, and it is never cheaper than the
    // linear arm the same entry point can also take. The gas gate pins exactly
    // one row per public entry point, so the two arms share this one.
    let (h, id) = fresh_curved(crate::ReleaseCurve::FrontLoaded);
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

    let (h, _) = fresh();
    h.client.set_halt_operator(&h.sender);
    record(&h, "set_halt_operator");

    let (h, _) = fresh();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    record(&h, "halt");

    let (h, _) = fresh();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    h.client.resume_contract();
    record(&h, "resume_contract");

    let (h, _) = fresh();
    h.client.set_halt_operator(&h.sender);
    h.client.halt();
    h.client.halted();
    record(&h, "halted");

    let (h, _) = fresh();
    h.client.set_halt_operator(&h.sender);
    h.client.halt_operator();
    record(&h, "halt_operator");
}
