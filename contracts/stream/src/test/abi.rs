//! Generated ABI inventory and additive-compatibility checks.
//!
//! # Binding format
//!
//! The inventory is JSON, generated from the `ScSpecEntry` XDR that
//! `#[contractimpl]`, `#[contracttype]`, `#[contracterror]` and
//! `#[contractevent]` already embed. Docs are stripped so a comment-only edit
//! does not churn the snapshot. Auth is not in the spec; it is labelled
//! explicitly per method (sender / recipient / none) and must cover every
//! public entrypoint.
//!
//! # Compatibility rules
//!
//! Matching `docs/ABI.md`:
//!
//! * **Additive** (no [`crate::ABI_VERSION`] bump): new method; new error
//!   discriminant appended; new field at the *end* of an event payload.
//! * **Breaking** (bump required): removed or renamed method; changed
//!   parameter type, order or count; changed return type; renamed struct
//!   field; reordered event topics; renumbered error discriminant.
//!
//! Failures are typed `Error` values. The contract does not retry; the caller
//! resubmits the transaction. Authorization is `require_auth` on the labelled
//! party, or none for views and TTL maintenance.

use std::collections::BTreeMap;
use std::string::{String, ToString};
use std::vec::Vec;
use std::{format, vec};

use soroban_sdk::xdr::{
    Limits, ReadXdr, ScSpecEntry, ScSpecEventParamLocationV0, ScSpecTypeDef, StringM,
};

use crate::events::{
    Cancelled, Paused, RecipientTransferred, Resumed, StreamCreated, ToppedUp, TtlExtended,
    Withdrawn,
};
use crate::{Error, FluxoraStream, Stream, StreamStatus, ABI_VERSION};

// ---------------------------------------------------------------------------
// Inventory
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inventory {
    abi_version: u32,
    functions: Vec<FunctionAbi>,
    types: Vec<TypeAbi>,
    errors: Vec<ErrorCase>,
    events: Vec<EventAbi>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FunctionAbi {
    name: String,
    auth: &'static str,
    inputs: Vec<ParamAbi>,
    outputs: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParamAbi {
    name: String,
    type_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TypeAbi {
    name: String,
    kind: String,
    fields: Vec<ParamAbi>,
    cases: Vec<ErrorCase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ErrorCase {
    name: String,
    discriminant: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EventAbi {
    name: String,
    topics: Vec<ParamAbi>,
    data: Vec<ParamAbi>,
}

/// Who `require_auth` is invoked on. `"none"` means permissionless.
///
/// Not derived from spec XDR — authorization is a body-level check — so the
/// table is the explicit record. Adding a public method without an entry here
/// fails the suite.
const AUTH: &[(&str, &str)] = &[
    ("create_stream", "sender"),
    ("top_up", "sender"),
    ("cancel", "sender"),
    ("pause", "sender"),
    ("resume", "sender"),
    ("withdraw", "recipient"),
    ("batch_withdraw", "recipient"),
    ("transfer_recipient", "recipient"),
    ("grant_delegate", "grantor"),
    ("revoke_delegate", "grantor"),
    ("delegate_withdraw", "delegate"),
    ("delegate_cancel", "delegate"),
    ("delegate_pause", "delegate"),
    ("delegate_resume", "delegate"),
    ("delegate_top_up", "delegate"),
    ("delegate_transfer_recipient", "delegate"),
    ("get_stream", "none"),
    ("withdrawable_of", "none"),
    ("vested_of", "none"),
    ("refundable_of", "none"),
    ("stream_count", "none"),
    ("stream_exists", "none"),
    ("extend_stream_ttl", "none"),
    ("batch_extend_ttl", "none"),
];

fn auth_of(name: &str) -> &'static str {
    AUTH.iter()
        .find(|(n, _)| *n == name)
        .map(|(_, a)| *a)
        .unwrap_or_else(|| panic!("public method `{name}` has no AUTH table entry"))
}

// ---------------------------------------------------------------------------
// Spec → inventory
// ---------------------------------------------------------------------------

fn parse_spec(bytes: &[u8]) -> ScSpecEntry {
    ScSpecEntry::from_xdr(bytes, Limits::none()).expect("embedded spec XDR is well-formed")
}

fn spec_ident<const N: u32>(name: &StringM<N>) -> String {
    String::from(std::str::from_utf8(name.as_ref()).expect("spec name is utf-8"))
}

fn format_type(t: &ScSpecTypeDef) -> String {
    match t {
        ScSpecTypeDef::Val => "Val".into(),
        ScSpecTypeDef::Bool => "bool".into(),
        ScSpecTypeDef::Void => "()".into(),
        ScSpecTypeDef::Error => "Error".into(),
        ScSpecTypeDef::U32 => "u32".into(),
        ScSpecTypeDef::I32 => "i32".into(),
        ScSpecTypeDef::U64 => "u64".into(),
        ScSpecTypeDef::I64 => "i64".into(),
        ScSpecTypeDef::Timepoint => "Timepoint".into(),
        ScSpecTypeDef::Duration => "Duration".into(),
        ScSpecTypeDef::U128 => "u128".into(),
        ScSpecTypeDef::I128 => "i128".into(),
        ScSpecTypeDef::U256 => "U256".into(),
        ScSpecTypeDef::I256 => "I256".into(),
        ScSpecTypeDef::Bytes => "Bytes".into(),
        ScSpecTypeDef::String => "String".into(),
        ScSpecTypeDef::Symbol => "Symbol".into(),
        ScSpecTypeDef::Address => "Address".into(),
        ScSpecTypeDef::MuxedAddress => "MuxedAddress".into(),
        ScSpecTypeDef::Option(inner) => format!("Option<{}>", format_type(&inner.value_type)),
        ScSpecTypeDef::Result(inner) => format!(
            "Result<{}, {}>",
            format_type(&inner.ok_type),
            format_type(&inner.error_type)
        ),
        ScSpecTypeDef::Vec(inner) => format!("Vec<{}>", format_type(&inner.element_type)),
        ScSpecTypeDef::Map(inner) => format!(
            "Map<{}, {}>",
            format_type(&inner.key_type),
            format_type(&inner.value_type)
        ),
        ScSpecTypeDef::Tuple(inner) => {
            let parts: Vec<String> = inner.value_types.iter().map(format_type).collect();
            format!("({})", parts.join(", "))
        }
        ScSpecTypeDef::BytesN(inner) => format!("BytesN<{}>", inner.n),
        ScSpecTypeDef::Udt(inner) => spec_ident(&inner.name),
    }
}

fn function_from_spec(entry: ScSpecEntry) -> FunctionAbi {
    let ScSpecEntry::FunctionV0(f) = entry else {
        panic!("expected FunctionV0 spec entry, got {entry:?}");
    };
    let name = spec_ident(&f.name);
    let inputs = f
        .inputs
        .iter()
        .map(|p| ParamAbi {
            name: spec_ident(&p.name),
            type_name: format_type(&p.type_),
        })
        .collect();
    let outputs = match f.outputs.first() {
        Some(t) => format_type(t),
        None => "()".into(),
    };
    FunctionAbi {
        auth: auth_of(&name),
        name,
        inputs,
        outputs,
    }
}

fn type_from_spec(entry: ScSpecEntry) -> TypeAbi {
    match entry {
        ScSpecEntry::UdtStructV0(s) => TypeAbi {
            name: spec_ident(&s.name),
            kind: "struct".into(),
            fields: s
                .fields
                .iter()
                .map(|f| ParamAbi {
                    name: spec_ident(&f.name),
                    type_name: format_type(&f.type_),
                })
                .collect(),
            cases: Vec::new(),
        },
        ScSpecEntry::UdtEnumV0(e) => TypeAbi {
            name: spec_ident(&e.name),
            kind: "enum".into(),
            fields: Vec::new(),
            cases: e
                .cases
                .iter()
                .map(|c| ErrorCase {
                    name: spec_ident(&c.name),
                    discriminant: c.value,
                })
                .collect(),
        },
        other => panic!("expected struct or enum spec entry, got {other:?}"),
    }
}

fn errors_from_spec(entry: ScSpecEntry) -> Vec<ErrorCase> {
    let ScSpecEntry::UdtErrorEnumV0(e) = entry else {
        panic!("expected error-enum spec entry, got {entry:?}");
    };
    e.cases
        .iter()
        .map(|c| ErrorCase {
            name: spec_ident(&c.name),
            discriminant: c.value,
        })
        .collect()
}

fn event_from_spec(entry: ScSpecEntry) -> EventAbi {
    let ScSpecEntry::EventV0(e) = entry else {
        panic!("expected EventV0 spec entry, got {entry:?}");
    };
    let mut topics = Vec::new();
    let mut data = Vec::new();
    for p in e.params.iter() {
        let param = ParamAbi {
            name: spec_ident(&p.name),
            type_name: format_type(&p.type_),
        };
        match p.location {
            ScSpecEventParamLocationV0::TopicList => topics.push(param),
            ScSpecEventParamLocationV0::Data => data.push(param),
        }
    }
    EventAbi {
        name: spec_ident(&e.name),
        topics,
        data,
    }
}

fn current_inventory() -> Inventory {
    let mut functions = vec![
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_create_stream())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_top_up())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_withdraw())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_batch_withdraw())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_cancel())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_pause())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_resume())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_transfer_recipient())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_get_stream())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_withdrawable_of())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_vested_of())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_refundable_of())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_stream_count())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_stream_exists())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_extend_stream_ttl())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_batch_extend_ttl())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_grant_delegate())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_revoke_delegate())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_delegate_withdraw())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_delegate_cancel())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_delegate_pause())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_delegate_resume())),
        function_from_spec(parse_spec(&FluxoraStream::spec_xdr_delegate_top_up())),
        function_from_spec(parse_spec(
            &FluxoraStream::spec_xdr_delegate_transfer_recipient(),
        )),
    ];
    functions.sort_by(|a, b| a.name.cmp(&b.name));

    let mut types = vec![
        type_from_spec(parse_spec(&Stream::spec_xdr())),
        type_from_spec(parse_spec(&StreamStatus::spec_xdr())),
    ];
    types.sort_by(|a, b| a.name.cmp(&b.name));

    let errors = errors_from_spec(parse_spec(&Error::spec_xdr()));

    let mut events = vec![
        event_from_spec(parse_spec(&StreamCreated::spec_xdr())),
        event_from_spec(parse_spec(&Withdrawn::spec_xdr())),
        event_from_spec(parse_spec(&Cancelled::spec_xdr())),
        event_from_spec(parse_spec(&Paused::spec_xdr())),
        event_from_spec(parse_spec(&Resumed::spec_xdr())),
        event_from_spec(parse_spec(&ToppedUp::spec_xdr())),
        event_from_spec(parse_spec(&RecipientTransferred::spec_xdr())),
        event_from_spec(parse_spec(&TtlExtended::spec_xdr())),
    ];
    events.sort_by(|a, b| a.name.cmp(&b.name));

    Inventory {
        abi_version: ABI_VERSION,
        functions,
        types,
        errors,
        events,
    }
}

// ---------------------------------------------------------------------------
// Frozen v1 baseline (docs/ABI.md, 2026-08-12)
// ---------------------------------------------------------------------------

fn param(name: &str, type_name: &str) -> ParamAbi {
    ParamAbi {
        name: name.into(),
        type_name: type_name.into(),
    }
}

fn fn_abi(
    name: &'static str,
    auth: &'static str,
    inputs: Vec<ParamAbi>,
    outputs: &str,
) -> FunctionAbi {
    FunctionAbi {
        name: name.into(),
        auth,
        inputs,
        outputs: outputs.into(),
    }
}

/// Interface of record. Edit only when bumping [`ABI_VERSION`].
fn frozen_v1() -> Inventory {
    let mut functions = vec![
        fn_abi(
            "create_stream",
            "sender",
            vec![
                param("sender", "Address"),
                param("recipient", "Address"),
                param("token", "Address"),
                param("deposit", "i128"),
                param("start_time", "u64"),
                param("end_time", "u64"),
                param("cliff_time", "u64"),
                param("cancellable", "bool"),
                param("pausable", "bool"),
                param("transferable", "bool"),
            ],
            "Result<u64, Error>",
        ),
        fn_abi(
            "top_up",
            "sender",
            vec![param("stream_id", "u64"), param("amount", "i128")],
            "Result<(), Error>",
        ),
        fn_abi(
            "withdraw",
            "recipient",
            vec![param("stream_id", "u64"), param("amount", "Option<i128>")],
            "Result<i128, Error>",
        ),
        fn_abi(
            "batch_withdraw",
            "recipient",
            vec![
                param("recipient", "Address"),
                param("stream_ids", "Vec<u64>"),
            ],
            "Result<i128, Error>",
        ),
        fn_abi(
            "cancel",
            "sender",
            vec![param("stream_id", "u64")],
            "Result<(), Error>",
        ),
        fn_abi(
            "pause",
            "sender",
            vec![param("stream_id", "u64")],
            "Result<(), Error>",
        ),
        fn_abi(
            "resume",
            "sender",
            vec![param("stream_id", "u64")],
            "Result<(), Error>",
        ),
        fn_abi(
            "transfer_recipient",
            "recipient",
            vec![param("stream_id", "u64"), param("new_recipient", "Address")],
            "Result<(), Error>",
        ),
        fn_abi(
            "get_stream",
            "none",
            vec![param("stream_id", "u64")],
            "Result<Stream, Error>",
        ),
        fn_abi(
            "withdrawable_of",
            "none",
            vec![param("stream_id", "u64")],
            "Result<i128, Error>",
        ),
        fn_abi(
            "vested_of",
            "none",
            vec![param("stream_id", "u64")],
            "Result<i128, Error>",
        ),
        fn_abi(
            "refundable_of",
            "none",
            vec![param("stream_id", "u64")],
            "Result<i128, Error>",
        ),
        fn_abi("stream_count", "none", vec![], "u64"),
        fn_abi(
            "stream_exists",
            "none",
            vec![param("stream_id", "u64")],
            "bool",
        ),
        fn_abi(
            "extend_stream_ttl",
            "none",
            vec![param("stream_id", "u64")],
            "Result<u32, Error>",
        ),
        fn_abi(
            "batch_extend_ttl",
            "none",
            vec![param("stream_ids", "Vec<u64>")],
            "Result<u32, Error>",
        ),
    ];
    functions.sort_by(|a, b| a.name.cmp(&b.name));

    let types = vec![
        TypeAbi {
            name: "Stream".into(),
            kind: "struct".into(),
            fields: vec![
                param("cancellable", "bool"),
                param("cliff_time", "u64"),
                param("deposited", "i128"),
                param("end_time", "u64"),
                param("pausable", "bool"),
                param("paused_at", "Option<u64>"),
                param("paused_total", "u64"),
                param("recipient", "Address"),
                param("sender", "Address"),
                param("start_time", "u64"),
                param("status", "StreamStatus"),
                param("token", "Address"),
                param("transferable", "bool"),
                param("withdrawn", "i128"),
            ],
            cases: Vec::new(),
        },
        TypeAbi {
            name: "StreamStatus".into(),
            kind: "enum".into(),
            fields: Vec::new(),
            cases: vec![
                ErrorCase {
                    name: "Active".into(),
                    discriminant: 0,
                },
                ErrorCase {
                    name: "Paused".into(),
                    discriminant: 1,
                },
                ErrorCase {
                    name: "Cancelled".into(),
                    discriminant: 2,
                },
                ErrorCase {
                    name: "Depleted".into(),
                    discriminant: 3,
                },
            ],
        },
    ];

    let events = vec![
        EventAbi {
            name: "Cancelled".into(),
            topics: vec![
                param("stream_id", "u64"),
                param("sender", "Address"),
                param("recipient", "Address"),
            ],
            data: vec![
                param("refunded", "i128"),
                param("vested", "i128"),
                param("withdrawn", "i128"),
                param("end_time", "u64"),
            ],
        },
        EventAbi {
            name: "Paused".into(),
            topics: vec![param("stream_id", "u64"), param("sender", "Address")],
            data: vec![param("paused_at", "u64"), param("paused_total", "u64")],
        },
        EventAbi {
            name: "RecipientTransferred".into(),
            topics: vec![
                param("stream_id", "u64"),
                param("old_recipient", "Address"),
                param("new_recipient", "Address"),
            ],
            data: Vec::new(),
        },
        EventAbi {
            name: "Resumed".into(),
            topics: vec![param("stream_id", "u64"), param("sender", "Address")],
            data: vec![
                param("paused_duration", "u64"),
                param("paused_total", "u64"),
            ],
        },
        EventAbi {
            name: "StreamCreated".into(),
            topics: vec![
                param("stream_id", "u64"),
                param("sender", "Address"),
                param("recipient", "Address"),
            ],
            data: vec![
                param("token", "Address"),
                param("deposited", "i128"),
                param("start_time", "u64"),
                param("end_time", "u64"),
                param("cliff_time", "u64"),
                param("cancellable", "bool"),
                param("pausable", "bool"),
                param("transferable", "bool"),
            ],
        },
        EventAbi {
            name: "ToppedUp".into(),
            topics: vec![param("stream_id", "u64"), param("sender", "Address")],
            data: vec![
                param("amount", "i128"),
                param("deposited", "i128"),
                param("end_time", "u64"),
            ],
        },
        EventAbi {
            name: "TtlExtended".into(),
            topics: vec![param("stream_id", "u64")],
            data: vec![param("extended_to_ledgers", "u32")],
        },
        EventAbi {
            name: "Withdrawn".into(),
            topics: vec![param("stream_id", "u64"), param("recipient", "Address")],
            data: vec![
                param("amount", "i128"),
                param("withdrawn", "i128"),
                param("deposited", "i128"),
                param("status", "StreamStatus"),
            ],
        },
    ];

    Inventory {
        abi_version: 1,
        functions,
        types,
        errors: vec![
            ErrorCase {
                name: "StreamNotFound".into(),
                discriminant: 1,
            },
            ErrorCase {
                name: "InvalidTimeRange".into(),
                discriminant: 2,
            },
            ErrorCase {
                name: "InvalidCliff".into(),
                discriminant: 3,
            },
            ErrorCase {
                name: "InvalidDeposit".into(),
                discriminant: 4,
            },
            ErrorCase {
                name: "DepositRateTooLow".into(),
                discriminant: 5,
            },
            ErrorCase {
                name: "SelfStream".into(),
                discriminant: 6,
            },
            ErrorCase {
                name: "Unauthorized".into(),
                discriminant: 7,
            },
            ErrorCase {
                name: "NotCancellable".into(),
                discriminant: 8,
            },
            ErrorCase {
                name: "NotPausable".into(),
                discriminant: 9,
            },
            ErrorCase {
                name: "NotTransferable".into(),
                discriminant: 10,
            },
            ErrorCase {
                name: "StreamNotActive".into(),
                discriminant: 11,
            },
            ErrorCase {
                name: "StreamNotPaused".into(),
                discriminant: 12,
            },
            ErrorCase {
                name: "StreamAlreadyPaused".into(),
                discriminant: 13,
            },
            ErrorCase {
                name: "StreamTerminated".into(),
                discriminant: 14,
            },
            ErrorCase {
                name: "StreamMatured".into(),
                discriminant: 15,
            },
            ErrorCase {
                name: "InsufficientWithdrawable".into(),
                discriminant: 16,
            },
            ErrorCase {
                name: "NothingToWithdraw".into(),
                discriminant: 17,
            },
            ErrorCase {
                name: "InvalidAmount".into(),
                discriminant: 18,
            },
            ErrorCase {
                name: "BatchTooLarge".into(),
                discriminant: 19,
            },
            ErrorCase {
                name: "EmptyBatch".into(),
                discriminant: 20,
            },
            ErrorCase {
                name: "DuplicateStreamId".into(),
                discriminant: 21,
            },
            ErrorCase {
                name: "Overflow".into(),
                discriminant: 22,
            },
            ErrorCase {
                name: "TopUpTooSmall".into(),
                discriminant: 23,
            },
        ],
        events,
    }
}

// ---------------------------------------------------------------------------
// Compatibility
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChangeKind {
    Additive,
    Breaking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Change {
    kind: ChangeKind,
    detail: String,
}

fn diff(old: &Inventory, new: &Inventory) -> Vec<Change> {
    let mut changes = Vec::new();
    diff_functions(old, new, &mut changes);
    diff_errors(old, new, &mut changes);
    diff_types(old, new, &mut changes);
    diff_events(old, new, &mut changes);
    changes
}

fn by_name_fn(items: &[FunctionAbi]) -> BTreeMap<&str, &FunctionAbi> {
    items.iter().map(|f| (f.name.as_str(), f)).collect()
}

fn diff_functions(old: &Inventory, new: &Inventory, changes: &mut Vec<Change>) {
    let before = by_name_fn(&old.functions);
    let after = by_name_fn(&new.functions);

    for (name, prev) in &before {
        match after.get(name) {
            None => changes.push(Change {
                kind: ChangeKind::Breaking,
                detail: format!("removed method `{name}`"),
            }),
            Some(next) => {
                if prev.inputs != next.inputs {
                    changes.push(Change {
                        kind: ChangeKind::Breaking,
                        detail: format!("type-changed method `{name}` inputs"),
                    });
                }
                if prev.outputs != next.outputs {
                    changes.push(Change {
                        kind: ChangeKind::Breaking,
                        detail: format!("type-changed method `{name}` return type"),
                    });
                }
                if prev.auth != next.auth {
                    changes.push(Change {
                        kind: ChangeKind::Breaking,
                        detail: format!("authorization changed on `{name}`"),
                    });
                }
            }
        }
    }

    for name in after.keys() {
        if !before.contains_key(name) {
            changes.push(Change {
                kind: ChangeKind::Additive,
                detail: format!("added method `{name}`"),
            });
        }
    }

    // A disappearance plus a new name in the same diff is a rename: still
    // breaking because clients address methods by name.
    let removed: Vec<_> = before
        .keys()
        .filter(|n| !after.contains_key(*n))
        .copied()
        .collect();
    let added: Vec<_> = after
        .keys()
        .filter(|n| !before.contains_key(*n))
        .copied()
        .collect();
    if removed.len() == 1 && added.len() == 1 {
        changes.push(Change {
            kind: ChangeKind::Breaking,
            detail: format!("renamed method `{}` -> `{}`", removed[0], added[0]),
        });
    }
}

fn diff_errors(old: &Inventory, new: &Inventory, changes: &mut Vec<Change>) {
    if old.errors.is_empty() {
        return;
    }
    let before: BTreeMap<&str, u32> = old
        .errors
        .iter()
        .map(|e| (e.name.as_str(), e.discriminant))
        .collect();
    let after: BTreeMap<&str, u32> = new
        .errors
        .iter()
        .map(|e| (e.name.as_str(), e.discriminant))
        .collect();

    for (name, disc) in &before {
        match after.get(name) {
            None => changes.push(Change {
                kind: ChangeKind::Breaking,
                detail: format!("removed error `{name}`"),
            }),
            Some(new_disc) if new_disc != disc => changes.push(Change {
                kind: ChangeKind::Breaking,
                detail: format!("renumbered error `{name}`: {disc} -> {new_disc}"),
            }),
            Some(_) => {}
        }
    }
    for name in after.keys() {
        if !before.contains_key(name) {
            changes.push(Change {
                kind: ChangeKind::Additive,
                detail: format!("added error `{name}`"),
            });
        }
    }
}

fn diff_types(old: &Inventory, new: &Inventory, changes: &mut Vec<Change>) {
    if old.types.is_empty() {
        return;
    }
    let before: BTreeMap<&str, &TypeAbi> = old.types.iter().map(|t| (t.name.as_str(), t)).collect();
    let after: BTreeMap<&str, &TypeAbi> = new.types.iter().map(|t| (t.name.as_str(), t)).collect();
    for (name, prev) in &before {
        match after.get(name) {
            None => changes.push(Change {
                kind: ChangeKind::Breaking,
                detail: format!("removed type `{name}`"),
            }),
            Some(next) if next.fields != prev.fields || next.cases != prev.cases => {
                changes.push(Change {
                    kind: ChangeKind::Breaking,
                    detail: format!("type-changed UDT `{name}`"),
                });
            }
            Some(_) => {}
        }
    }
}

fn diff_events(old: &Inventory, new: &Inventory, changes: &mut Vec<Change>) {
    if old.events.is_empty() {
        return;
    }
    let before: BTreeMap<&str, &EventAbi> =
        old.events.iter().map(|e| (e.name.as_str(), e)).collect();
    let after: BTreeMap<&str, &EventAbi> =
        new.events.iter().map(|e| (e.name.as_str(), e)).collect();
    for (name, prev) in &before {
        match after.get(name) {
            None => changes.push(Change {
                kind: ChangeKind::Breaking,
                detail: format!("removed event `{name}`"),
            }),
            Some(next) => {
                if next.topics != prev.topics {
                    changes.push(Change {
                        kind: ChangeKind::Breaking,
                        detail: format!("reordered or changed topics on event `{name}`"),
                    });
                }
                if next.data.len() < prev.data.len()
                    || next.data[..prev.data.len().min(next.data.len())]
                        != prev.data[..prev.data.len().min(next.data.len())]
                {
                    changes.push(Change {
                        kind: ChangeKind::Breaking,
                        detail: format!("changed event payload `{name}`"),
                    });
                } else if next.data.len() > prev.data.len() {
                    changes.push(Change {
                        kind: ChangeKind::Additive,
                        detail: format!("appended event payload fields on `{name}`"),
                    });
                }
            }
        }
    }
}

fn check_compatibility(old: &Inventory, new: &Inventory) -> Result<(), String> {
    if new.abi_version < old.abi_version {
        return Err(format!(
            "ABI_VERSION downgrade is not allowed ({} -> {})",
            old.abi_version, new.abi_version
        ));
    }

    let changes = diff(old, new);
    let breaking: Vec<&Change> = changes
        .iter()
        .filter(|c| c.kind == ChangeKind::Breaking)
        .collect();

    if !breaking.is_empty() && new.abi_version <= old.abi_version {
        let details: Vec<&str> = breaking.iter().map(|c| c.detail.as_str()).collect();
        return Err(format!(
            "breaking ABI change without ABI_VERSION bump (still {}): {}",
            new.abi_version,
            details.join("; ")
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// JSON renderer (no extra crate — dependency churn is out of scope)
// ---------------------------------------------------------------------------

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_json(inv: &Inventory) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"abi_version\": {},\n", inv.abi_version));
    out.push_str("  \"functions\": [\n");
    for (i, f) in inv.functions.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{}\",\n", escape(&f.name)));
        out.push_str(&format!("      \"auth\": \"{}\",\n", f.auth));
        out.push_str("      \"inputs\": [\n");
        for (j, p) in f.inputs.iter().enumerate() {
            let comma = if j + 1 == f.inputs.len() { "" } else { "," };
            out.push_str(&format!(
                "        {{\"name\": \"{}\", \"type\": \"{}\"}}{}\n",
                escape(&p.name),
                escape(&p.type_name),
                comma
            ));
        }
        out.push_str("      ],\n");
        out.push_str(&format!("      \"outputs\": \"{}\"\n", escape(&f.outputs)));
        out.push_str(if i + 1 == inv.functions.len() {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ],\n");

    out.push_str("  \"types\": [\n");
    for (i, t) in inv.types.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{}\",\n", escape(&t.name)));
        out.push_str(&format!("      \"kind\": \"{}\",\n", escape(&t.kind)));
        out.push_str("      \"fields\": [\n");
        for (j, p) in t.fields.iter().enumerate() {
            let comma = if j + 1 == t.fields.len() { "" } else { "," };
            out.push_str(&format!(
                "        {{\"name\": \"{}\", \"type\": \"{}\"}}{}\n",
                escape(&p.name),
                escape(&p.type_name),
                comma
            ));
        }
        out.push_str("      ],\n");
        out.push_str("      \"cases\": [\n");
        for (j, c) in t.cases.iter().enumerate() {
            let comma = if j + 1 == t.cases.len() { "" } else { "," };
            out.push_str(&format!(
                "        {{\"name\": \"{}\", \"discriminant\": {}}}{}\n",
                escape(&c.name),
                c.discriminant,
                comma
            ));
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == inv.types.len() {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ],\n");

    out.push_str("  \"errors\": [\n");
    for (i, e) in inv.errors.iter().enumerate() {
        let comma = if i + 1 == inv.errors.len() { "" } else { "," };
        out.push_str(&format!(
            "    {{\"name\": \"{}\", \"discriminant\": {}}}{}\n",
            escape(&e.name),
            e.discriminant,
            comma
        ));
    }
    out.push_str("  ],\n");

    out.push_str("  \"events\": [\n");
    for (i, e) in inv.events.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": \"{}\",\n", escape(&e.name)));
        out.push_str("      \"topics\": [\n");
        for (j, p) in e.topics.iter().enumerate() {
            let comma = if j + 1 == e.topics.len() { "" } else { "," };
            out.push_str(&format!(
                "        {{\"name\": \"{}\", \"type\": \"{}\"}}{}\n",
                escape(&p.name),
                escape(&p.type_name),
                comma
            ));
        }
        out.push_str("      ],\n");
        out.push_str("      \"data\": [\n");
        for (j, p) in e.data.iter().enumerate() {
            let comma = if j + 1 == e.data.len() { "" } else { "," };
            out.push_str(&format!(
                "        {{\"name\": \"{}\", \"type\": \"{}\"}}{}\n",
                escape(&p.name),
                escape(&p.type_name),
                comma
            ));
        }
        out.push_str("      ]\n");
        out.push_str(if i + 1 == inv.events.len() {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn pub_fn_names_from_lib() -> Vec<String> {
    include_str!("../lib.rs")
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("pub fn ")
                .and_then(|rest| rest.split('(').next())
                .map(|s| s.trim().to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn abi_snapshot_matches_generated_spec() {
    let generated = render_json(&current_inventory());
    let committed = include_str!("../../abi/fluxora_stream.json").replace('\r', "");
    if generated == committed {
        return;
    }
    if std::env::var("FLUXORA_UPDATE_ABI").is_ok() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("abi/fluxora_stream.json");
        std::fs::write(&path, generated.as_bytes()).expect("write ABI snapshot");
        panic!(
            "wrote {}; re-run without FLUXORA_UPDATE_ABI",
            path.display()
        );
    }
    assert_eq!(
        generated, committed,
        "ABI snapshot is stale. Update contracts/stream/abi/fluxora_stream.json (FLUXORA_UPDATE_ABI=1 cargo test -p fluxora-stream abi_snapshot)."
    );
}

#[test]
fn current_spec_is_compatible_with_frozen_v1() {
    check_compatibility(&frozen_v1(), &current_inventory()).unwrap();
}

#[test]
fn every_lib_rs_pub_fn_is_in_the_inventory() {
    let from_src = pub_fn_names_from_lib();
    let from_spec: Vec<String> = current_inventory()
        .functions
        .iter()
        .map(|f| f.name.clone())
        .collect();
    let mut src_sorted = from_src.clone();
    src_sorted.sort();
    assert_eq!(
        src_sorted, from_spec,
        "lib.rs pub fn set drifted from the generated spec inventory"
    );
}

#[test]
fn auth_table_covers_every_public_method() {
    let inv = current_inventory();
    let mut table: Vec<&str> = AUTH.iter().map(|(n, _)| *n).collect();
    table.sort();
    let names: Vec<&str> = inv.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(table, names);
    for f in &inv.functions {
        assert!(
            matches!(
                f.auth,
                "sender" | "recipient" | "grantor" | "delegate" | "none"
            ),
            "method {} has unknown auth {}",
            f.name,
            f.auth
        );
    }
}

#[test]
fn frozen_v1_error_discriminants_are_an_unchanged_prefix() {
    let current = current_inventory();
    let frozen = frozen_v1();
    // The frozen v1 set is the interface of record: existing discriminants
    // must be byte-for-byte identical, in order. New discriminants may only be
    // appended after it (additive per docs/ABI.md).
    assert!(current.errors.len() >= frozen.errors.len());
    assert_eq!(current.errors[..frozen.errors.len()], frozen.errors[..]);
}

#[test]
fn removing_a_method_without_a_version_bump_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.functions.retain(|f| f.name != "withdraw");
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(err.contains("removed method `withdraw`"), "{err}");
    assert!(err.contains("ABI_VERSION"), "{err}");
}

#[test]
fn renaming_a_method_without_a_version_bump_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    let f = new
        .functions
        .iter_mut()
        .find(|f| f.name == "withdraw")
        .unwrap();
    f.name = "claim".into();
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(
        err.contains("renamed method `withdraw` -> `claim`")
            || err.contains("removed method `withdraw`"),
        "{err}"
    );
}

#[test]
fn type_changing_a_parameter_without_a_version_bump_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    let f = new
        .functions
        .iter_mut()
        .find(|f| f.name == "withdraw")
        .unwrap();
    f.inputs[1].type_name = "i128".into();
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(
        err.contains("type-changed method `withdraw` inputs"),
        "{err}"
    );
}

#[test]
fn type_changing_a_return_without_a_version_bump_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    let f = new
        .functions
        .iter_mut()
        .find(|f| f.name == "stream_count")
        .unwrap();
    f.outputs = "u32".into();
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(
        err.contains("type-changed method `stream_count` return type"),
        "{err}"
    );
}

#[test]
fn adding_a_method_is_additive_and_needs_no_version_bump() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.functions.push(fn_abi(
        "rate_of",
        "none",
        vec![param("stream_id", "u64")],
        "Result<i128, Error>",
    ));
    new.functions.sort_by(|a, b| a.name.cmp(&b.name));
    check_compatibility(&old, &new).unwrap();
}

#[test]
fn breaking_change_is_accepted_once_the_version_bumps() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.functions.retain(|f| f.name != "withdraw");
    new.abi_version = old.abi_version + 1;
    check_compatibility(&old, &new).unwrap();
}

#[test]
fn version_downgrade_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.abi_version = 0;
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(err.contains("downgrade"), "{err}");
}

#[test]
fn renaming_a_stream_field_without_a_version_bump_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.types[0].fields[0].name = "can_cancel".into();
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(err.contains("type-changed UDT `Stream`"), "{err}");
}

#[test]
fn appending_an_event_payload_field_is_additive() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.events
        .iter_mut()
        .find(|e| e.name == "Withdrawn")
        .unwrap()
        .data
        .push(param("token", "Address"));
    check_compatibility(&old, &new).unwrap();
}

#[test]
fn appending_an_error_discriminant_is_additive() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.errors.push(ErrorCase {
        name: "NewFailure".into(),
        discriminant: 24,
    });
    check_compatibility(&old, &new).unwrap();
}

#[test]
fn renumbering_an_error_without_a_version_bump_is_rejected() {
    let old = frozen_v1();
    let mut new = old.clone();
    new.errors[0].discriminant = 99;
    let err = check_compatibility(&old, &new).unwrap_err();
    assert!(err.contains("renumbered error `StreamNotFound`"), "{err}");
}

/// Views and TTL maintenance are permissionless: they must succeed with an
/// empty auth context. Mutations must not. This is the ABI-level statement
/// of the existing `test::auth` coverage, keyed off the inventory labels.
#[test]
fn permissionless_methods_are_exactly_the_none_auth_set() {
    let none: Vec<&str> = AUTH
        .iter()
        .filter(|(_, a)| *a == "none")
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(
        none,
        [
            "get_stream",
            "withdrawable_of",
            "vested_of",
            "refundable_of",
            "stream_count",
            "stream_exists",
            "extend_stream_ttl",
            "batch_extend_ttl",
        ]
    );
}

#[test]
fn missing_stream_failure_is_stream_not_found_discriminant_one() {
    use super::common::*;
    use crate::Error;

    let h = Harness::new();
    let err = h.client.try_get_stream(&99).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamNotFound);
    assert_eq!(err as u32, 1);
}

#[test]
fn oversized_batch_failure_is_batch_too_large_discriminant_nineteen() {
    use super::common::*;
    use crate::Error;

    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..17).collect();
    let err = h
        .client
        .try_batch_extend_ttl(&h.ids(&ids))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::BatchTooLarge);
    assert_eq!(err as u32, 19);
}
