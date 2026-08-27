//! Internal, non-ABI modules that back the `#[contractimpl]` block in
//! `lib.rs`. Every item here is `pub(crate)` — the only public contract
//! surface remains the single `impl FluxoraStream` block in `lib.rs`.
//!
//! See issue #1520. Slice 1 introduces `validation`; later slices will add
//! per-lifecycle-operation modules (create, withdraw, cancel, pause_resume,
//! recipient, schedule, templates, auto_claim, clone, id_reservation,
//! offers, admin, views) following the same pattern.

pub(crate) mod validation;

// Slice 2+ modules — same pattern as slice 1 (issue #1520).
pub(crate) mod admin;
pub(crate) mod auto_claim;
pub(crate) mod auto_renew;
pub(crate) mod cancel;
pub(crate) mod clone;
pub(crate) mod create;
pub(crate) mod id_reservation;
pub(crate) mod offers;
pub(crate) mod pause_resume;
pub(crate) mod rate;
pub(crate) mod recipient;
pub(crate) mod schedule;
pub(crate) mod templates;
pub(crate) mod version;
pub(crate) mod views;
pub(crate) mod withdraw;
