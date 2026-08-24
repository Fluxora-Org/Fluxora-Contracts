//! Internal, non-ABI modules that back the `#[contractimpl]` block in
//! `lib.rs`. Every item here is `pub(crate)` — the only public contract
//! surface remains the single `impl FluxoraStream` block in `lib.rs`.
//!
//! See issue #1520. Slice 1 introduces `validation`; later slices will add
//! per-lifecycle-operation modules (create, withdraw, cancel, pause_resume,
//! recipient, schedule, templates, auto_claim, clone, id_reservation,
//! offers, admin, views) following the same pattern.

pub(crate) mod validation;
