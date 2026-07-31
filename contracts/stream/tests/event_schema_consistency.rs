extern crate std;

use std::collections::HashMap;

/// A single field in an event struct.
#[derive(Debug, PartialEq, Clone)]
struct EventField {
    name: String,
    typ: String,
}

/// Shape of an event payload struct.
#[derive(Debug, Clone)]
struct EventSchema {
    struct_name: String,
    fields: Vec<EventField>,
}

// ---------------------------------------------------------------------------
// Registry — hand-maintained list of every event payload struct in the
// stream contract (lib.rs / types.rs).
//
// When adding or modifying event structs you MUST:
//   1. Update the registry below.
//   2. Update docs/events.md with the new shape.
//   3. Run this test to confirm they match.
// ---------------------------------------------------------------------------

fn stream_event_registry() -> Vec<EventSchema> {
    vec![
        EventSchema {
            struct_name: "StreamCreated".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("sender", "Address"),
                field("recipient", "Address"),
                field("deposit_amount", "i128"),
                field("rate_per_second", "i128"),
                field("start_time", "u64"),
                field("cliff_time", "u64"),
                field("end_time", "u64"),
                field("withdraw_dust_threshold", "i128"),
                field("memo", "Option<Bytes>"),
                field("metadata", "Option<Map<Bytes,Bytes>>"),
            ],
        },
        EventSchema {
            struct_name: "StreamCloned".into(),
            fields: vec![
                field("new_stream_id", "u64"),
                field("source_stream_id", "u64"),
                field("sender", "Address"),
                field("recipient", "Address"),
                field("deposit_amount", "i128"),
                field("rate_per_second", "i128"),
                field("start_time", "u64"),
                field("cliff_time", "u64"),
                field("end_time", "u64"),
                field("withdraw_dust_threshold", "i128"),
            ],
        },
        EventSchema {
            struct_name: "Withdrawal".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("recipient", "Address"),
                field("amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "WithdrawalTo".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("recipient", "Address"),
                field("destination", "Address"),
                field("amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "RecipientUpdated".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_recipient", "Address"),
                field("new_recipient", "Address"),
            ],
        },
        EventSchema {
            struct_name: "RateUpdated".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_rate_per_second", "i128"),
                field("new_rate_per_second", "i128"),
                field("effective_time", "u64"),
            ],
        },
        EventSchema {
            struct_name: "RateCapEnforced".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("attempted_rate", "i128"),
                field("max_rate_per_second", "i128"),
            ],
        },
        EventSchema {
            struct_name: "RateDecreased".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_rate_per_second", "i128"),
                field("new_rate_per_second", "i128"),
                field("effective_time", "u64"),
                field("checkpointed_amount", "i128"),
                field("refund_amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamEndShortened".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_end_time", "u64"),
                field("new_end_time", "u64"),
                field("refund_amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamEndExtended".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_end_time", "u64"),
                field("new_end_time", "u64"),
            ],
        },
        EventSchema {
            struct_name: "StreamToppedUp".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("top_up_amount", "i128"),
                field("new_deposit_amount", "i128"),
                field("new_end_time", "u64"),
            ],
        },
        EventSchema {
            struct_name: "StreamRenewed".into(),
            fields: vec![field("old_stream_id", "u64"), field("new_stream_id", "u64")],
        },
        EventSchema {
            struct_name: "SenderTransferred".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_sender", "Address"),
                field("new_sender", "Address"),
            ],
        },
        EventSchema {
            struct_name: "StreamHealthChanged".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("is_underfunded", "bool"),
                field("remaining_balance", "i128"),
                field("seconds_remaining", "u64"),
            ],
        },
        EventSchema {
            struct_name: "GlobalEmergencyPauseChanged".into(),
            fields: vec![field("paused", "bool")],
        },
        EventSchema {
            struct_name: "ExcessSwept".into(),
            fields: vec![field("to", "Address"), field("amount", "i128")],
        },
        EventSchema {
            struct_name: "KeeperCancelled".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("keeper", "Address"),
                field("keeper_fee", "i128"),
                field("recipient_amount", "i128"),
                field("sender_refund", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamPaused".into(),
            fields: vec![field("stream_id", "u64"), field("reason", "String")],
        },
        EventSchema {
            struct_name: "GlobalResumed".into(),
            fields: vec![field("resumed_at", "u64")],
        },
        EventSchema {
            struct_name: "ContractPauseChanged".into(),
            fields: vec![field("paused", "bool")],
        },
        EventSchema {
            struct_name: "ProtocolPaused".into(),
            fields: vec![field("reason", "String"), field("paused_at", "u64")],
        },
        EventSchema {
            struct_name: "ProtocolResumed".into(),
            fields: vec![field("resumed_at", "u64")],
        },
        EventSchema {
            struct_name: "AutoClaimSet".into(),
            fields: vec![field("stream_id", "u64"), field("destination", "Address")],
        },
        EventSchema {
            struct_name: "AutoClaimRevoked".into(),
            fields: vec![field("stream_id", "u64")],
        },
        EventSchema {
            struct_name: "AutoClaimTriggered".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("destination", "Address"),
                field("amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "ClaimOwnershipTransferred".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_owner", "Option<Address>"),
                field("new_owner", "Address"),
            ],
        },
        EventSchema {
            struct_name: "StreamDecommissioned".into(),
            fields: vec![field("stream_id", "u64"), field("decommissioned", "bool")],
        },
        EventSchema {
            struct_name: "RecipientShareDelegated".into(),
            fields: vec![
                field("parent_stream_id", "u64"),
                field("child_stream_id", "u64"),
                field("delegator", "Address"),
                field("delegatee", "Address"),
                field("share_bps", "u32"),
                field("new_parent_rate", "i128"),
                field("child_rate", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamOfferCreated".into(),
            fields: vec![
                field("offer_id", "u64"),
                field("sender", "Address"),
                field("recipient", "Address"),
                field("deposit_amount", "i128"),
                field("rate_per_second", "i128"),
                field("start_time", "u64"),
                field("cliff_time", "u64"),
                field("end_time", "u64"),
                field("expiry_time", "Option<u64>"),
                field("created_at", "u64"),
            ],
        },
        EventSchema {
            struct_name: "StreamOfferAccepted".into(),
            fields: vec![
                field("offer_id", "u64"),
                field("effective_start_time", "u64"),
                field("recipient", "Address"),
            ],
        },
        EventSchema {
            struct_name: "StreamOfferCancelled".into(),
            fields: vec![
                field("offer_id", "u64"),
                field("by", "Address"),
                field("refund_amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "ContractUpgraded".into(),
            fields: vec![
                field("new_wasm_hash", "BytesN<32>"),
                field("new_version", "u32"),
                field("upgraded_at", "u64"),
                field("upgraded_by", "Address"),
            ],
        },
    ]
}

fn field(name: &str, typ: &str) -> EventField {
    EventField {
        name: name.into(),
        typ: typ.into(),
    }
}

// ---------------------------------------------------------------------------
// Doc parser — extracts struct definitions from docs/events.md
// ---------------------------------------------------------------------------

/// Normalise a type string for comparison (strip module prefixes, trim whitespace).
fn normalize_type(raw: &str) -> String {
    let s = raw.trim();
    // Strip leading colons and common prefixes
    let s = s.replace("soroban_sdk::", "").replace("crate::", "");
    // Collapse whitespace inside generics
    let s = s.replace(" >", ">").replace("> ", ">");
    let s = s.replace(" ,", ",").replace(", ", ",");
    let s = s.replace("< ", "<").replace(" <", "<");
    s.trim().to_string()
}

/// Parse lines inside a fenced code block for `pub struct <Name>` definitions.
fn parse_struct_from_code_block(lines: &[String]) -> Option<EventSchema> {
    let text: String = lines.join("\n");
    let text = text.as_str();

    // Match: `pub struct Name {` ... `}`
    let start_marker = "pub struct ";
    let si = text.find(start_marker)?;
    let after_start = &text[si + start_marker.len()..];

    let name_end = after_start.find(|c: char| c.is_whitespace() || c == '{')?;
    let struct_name = after_start[..name_end].trim().to_string();

    let brace_open = text[si..].find('{')?;
    let brace_close = text[si..].rfind('}')?;
    let body = &text[si + brace_open + 1..si + brace_close];

    let mut fields = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('}') {
            continue;
        }
        // Match: `pub name: type,` or `name: type,` or `pub name: type`
        let field_line = line.strip_suffix(',').unwrap_or(line).trim();
        if field_line.starts_with("pub ") {
            let after_pub = &field_line[4..];
            if let Some(colon) = after_pub.find(':') {
                let name = after_pub[..colon].trim();
                let typ = after_pub[colon + 1..].trim();
                // Skip everything after // comment
                let typ = typ.split("//").next().unwrap_or(typ).trim();
                fields.push(EventField {
                    name: name.to_string(),
                    typ: normalize_type(typ),
                });
            }
        }
    }

    Some(EventSchema {
        struct_name,
        fields,
    })
}

/// Parse lines inside a fenced code block for `pub enum <Name>` with tuple variants.
fn parse_enum_from_code_block(lines: &[String]) -> Option<EventSchema> {
    let text: String = lines.join("\n");
    let text = text.as_str();

    let start_marker = "pub enum ";
    let si = text.find(start_marker)?;
    let after_start = &text[si + start_marker.len()..];

    let name_end = after_start.find(|c: char| c.is_whitespace() || c == '{')?;
    let enum_name = after_start[..name_end].trim().to_string();

    if enum_name != "StreamEvent" {
        return None;
    }

    let brace_open = text[si..].find('{')?;
    let brace_close = text[si..].rfind('}')?;
    let body = &text[si + brace_open + 1..si + brace_close];

    let mut fields = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('}') {
            continue;
        }
        let variant_line = line.strip_suffix(',').unwrap_or(line).trim();
        // Match: `Variant(type)` — extract the variant name and inner type
        if let Some(paren_open) = variant_line.find('(') {
            let variant_name = variant_line[..paren_open].trim().to_string();
            let inner = &variant_line[paren_open + 1..];
            if let Some(paren_close) = inner.rfind(')') {
                let inner_type = inner[..paren_close].trim();
                fields.push(EventField {
                    name: variant_name,
                    typ: normalize_type(inner_type),
                });
            }
        }
    }

    Some(EventSchema {
        struct_name: enum_name,
        fields,
    })
}

/// Find struct definitions in `data: StructName { ... }` inline blocks.
fn parse_inline_struct(lines: &[String]) -> Option<EventSchema> {
    let text: String = lines.join("\n");
    let text = text.as_str();

    // Look for pattern: `data:   Name {`
    let data_marker = "data:";
    let di = text.find(data_marker)?;
    let after_data = text[di + data_marker.len()..].trim_start();

    // Find struct name before {
    let brace_open = after_data.find('{')?;
    let struct_name = after_data[..brace_open].trim().to_string();

    if !struct_name.chars().next()?.is_uppercase() {
        return None;
    }

    // Extract the brace block
    let inner_start = di + data_marker.len() + brace_open + 1;
    let mut depth = 1;
    let mut inner_end = inner_start;
    for (i, c) in text[inner_start..].char_indices() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                inner_end = inner_start + i;
                break;
            }
        }
    }
    if depth != 0 {
        return None;
    }
    let body = &text[inner_start..inner_end];

    let mut fields = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fline = line.strip_suffix(',').unwrap_or(line).trim();
        // Match: `name: type` with optional leading // comment
        if let Some(colon) = fline.find(':') {
            let name = fline[..colon].trim();
            // Skip field names that start with // or are empty
            if name.is_empty() || name.starts_with('/') {
                continue;
            }
            let typ_raw = fline[colon + 1..].trim();
            // Strip inline comments
            let typ = typ_raw.split("//").next().unwrap_or(typ_raw).trim();
            fields.push(EventField {
                name: name.to_string(),
                typ: normalize_type(typ),
            });
        }
    }

    if fields.is_empty() {
        return None;
    }

    Some(EventSchema {
        struct_name,
        fields,
    })
}

/// Collect all struct definitions from docs/events.md content.
fn parse_doc_schemas(doc: &str) -> Vec<EventSchema> {
    let mut result = Vec::new();

    let lines: Vec<String> = doc.lines().map(|l| l.to_string()).collect();
    let mut i = 0;
    let mut in_fence = false;
    let mut fence_lines: Vec<String> = Vec::new();

    while i < lines.len() {
        let line = lines[i].trim().to_string();

        // Detect fenced code blocks (``` or ```rust)
        if line.starts_with("```") {
            if in_fence {
                // End of fenced block — try to parse
                if let Some(schema) = parse_struct_from_code_block(&fence_lines) {
                    result.push(schema);
                }
                if let Some(schema) = parse_enum_from_code_block(&fence_lines) {
                    result.push(schema);
                }
                fence_lines.clear();
                in_fence = false;
            } else {
                in_fence = true;
                fence_lines.clear();
            }
            i += 1;
            continue;
        }

        if in_fence {
            fence_lines.push(lines[i].clone());
            i += 1;
            continue;
        }

        i += 1;
    }

    // Second pass: parse inline `data: StructName {` blocks
    // Process in groups separated by blank lines
    let mut block_lines: Vec<String> = Vec::new();
    for line in doc.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() && !block_lines.is_empty() {
            if let Some(schema) = parse_inline_struct(&block_lines) {
                // Avoid duplicates
                if !result.iter().any(|s| s.struct_name == schema.struct_name) {
                    result.push(schema);
                }
            }
            block_lines.clear();
        } else if !trimmed.is_empty() {
            block_lines.push(line.to_string());
        }
    }
    // Last block
    if !block_lines.is_empty() {
        if let Some(schema) = parse_inline_struct(&block_lines) {
            if !result.iter().any(|s| s.struct_name == schema.struct_name) {
                result.push(schema);
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Additive-only evolution tests
// ---------------------------------------------------------------------------

/// Baseline registry snapshot — a **hardcoded copy** of the canonical field list
/// for every event struct at the current version. Used by the additive-only tests
/// below to detect regressions (field removal, reordering, or type change).
///
/// This MUST be a literal copy (not a delegate to `stream_event_registry()`) so
/// that the comparison between baseline and current registry actually detects
/// changes. When a new optional field is legitimately appended to an existing
/// event struct, update BOTH this baseline AND `stream_event_registry()` AND
/// `docs/events.md`.
fn baseline_event_registry() -> Vec<EventSchema> {
    vec![
        EventSchema {
            struct_name: "StreamCreated".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("sender", "Address"),
                field("recipient", "Address"),
                field("deposit_amount", "i128"),
                field("rate_per_second", "i128"),
                field("start_time", "u64"),
                field("cliff_time", "u64"),
                field("end_time", "u64"),
                field("withdraw_dust_threshold", "i128"),
                field("memo", "Option<Bytes>"),
                field("metadata", "Option<Map<Bytes,Bytes>>"),
            ],
        },
        EventSchema {
            struct_name: "StreamCloned".into(),
            fields: vec![
                field("new_stream_id", "u64"),
                field("source_stream_id", "u64"),
                field("sender", "Address"),
                field("recipient", "Address"),
                field("deposit_amount", "i128"),
                field("rate_per_second", "i128"),
                field("start_time", "u64"),
                field("cliff_time", "u64"),
                field("end_time", "u64"),
                field("withdraw_dust_threshold", "i128"),
            ],
        },
        EventSchema {
            struct_name: "Withdrawal".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("recipient", "Address"),
                field("amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "WithdrawalTo".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("recipient", "Address"),
                field("destination", "Address"),
                field("amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "RecipientUpdated".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_recipient", "Address"),
                field("new_recipient", "Address"),
            ],
        },
        EventSchema {
            struct_name: "RateUpdated".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_rate_per_second", "i128"),
                field("new_rate_per_second", "i128"),
                field("effective_time", "u64"),
            ],
        },
        EventSchema {
            struct_name: "RateCapEnforced".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("attempted_rate", "i128"),
                field("max_rate_per_second", "i128"),
            ],
        },
        EventSchema {
            struct_name: "RateDecreased".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_rate_per_second", "i128"),
                field("new_rate_per_second", "i128"),
                field("effective_time", "u64"),
                field("checkpointed_amount", "i128"),
                field("refund_amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamEndShortened".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_end_time", "u64"),
                field("new_end_time", "u64"),
                field("refund_amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamEndExtended".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_end_time", "u64"),
                field("new_end_time", "u64"),
            ],
        },
        EventSchema {
            struct_name: "StreamToppedUp".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("top_up_amount", "i128"),
                field("new_deposit_amount", "i128"),
                field("new_end_time", "u64"),
            ],
        },
        EventSchema {
            struct_name: "StreamRenewed".into(),
            fields: vec![field("old_stream_id", "u64"), field("new_stream_id", "u64")],
        },
        EventSchema {
            struct_name: "SenderTransferred".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_sender", "Address"),
                field("new_sender", "Address"),
            ],
        },
        EventSchema {
            struct_name: "StreamHealthChanged".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("is_underfunded", "bool"),
                field("remaining_balance", "i128"),
                field("seconds_remaining", "u64"),
            ],
        },
        EventSchema {
            struct_name: "GlobalEmergencyPauseChanged".into(),
            fields: vec![field("paused", "bool")],
        },
        EventSchema {
            struct_name: "ExcessSwept".into(),
            fields: vec![field("to", "Address"), field("amount", "i128")],
        },
        EventSchema {
            struct_name: "KeeperCancelled".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("keeper", "Address"),
                field("keeper_fee", "i128"),
                field("recipient_amount", "i128"),
                field("sender_refund", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamPaused".into(),
            fields: vec![field("stream_id", "u64"), field("reason", "String")],
        },
        EventSchema {
            struct_name: "GlobalResumed".into(),
            fields: vec![field("resumed_at", "u64")],
        },
        EventSchema {
            struct_name: "ContractPauseChanged".into(),
            fields: vec![field("paused", "bool")],
        },
        EventSchema {
            struct_name: "ProtocolPaused".into(),
            fields: vec![field("reason", "String"), field("paused_at", "u64")],
        },
        EventSchema {
            struct_name: "ProtocolResumed".into(),
            fields: vec![field("resumed_at", "u64")],
        },
        EventSchema {
            struct_name: "AutoClaimSet".into(),
            fields: vec![field("stream_id", "u64"), field("destination", "Address")],
        },
        EventSchema {
            struct_name: "AutoClaimRevoked".into(),
            fields: vec![field("stream_id", "u64")],
        },
        EventSchema {
            struct_name: "AutoClaimTriggered".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("destination", "Address"),
                field("amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "ClaimOwnershipTransferred".into(),
            fields: vec![
                field("stream_id", "u64"),
                field("old_owner", "Option<Address>"),
                field("new_owner", "Address"),
            ],
        },
        EventSchema {
            struct_name: "StreamDecommissioned".into(),
            fields: vec![field("stream_id", "u64"), field("decommissioned", "bool")],
        },
        EventSchema {
            struct_name: "RecipientShareDelegated".into(),
            fields: vec![
                field("parent_stream_id", "u64"),
                field("child_stream_id", "u64"),
                field("delegator", "Address"),
                field("delegatee", "Address"),
                field("share_bps", "u32"),
                field("new_parent_rate", "i128"),
                field("child_rate", "i128"),
            ],
        },
        EventSchema {
            struct_name: "StreamOfferCreated".into(),
            fields: vec![
                field("offer_id", "u64"),
                field("sender", "Address"),
                field("recipient", "Address"),
                field("deposit_amount", "i128"),
                field("rate_per_second", "i128"),
                field("start_time", "u64"),
                field("cliff_time", "u64"),
                field("end_time", "u64"),
                field("expiry_time", "Option<u64>"),
                field("created_at", "u64"),
            ],
        },
        EventSchema {
            struct_name: "StreamOfferAccepted".into(),
            fields: vec![
                field("offer_id", "u64"),
                field("effective_start_time", "u64"),
                field("recipient", "Address"),
            ],
        },
        EventSchema {
            struct_name: "StreamOfferCancelled".into(),
            fields: vec![
                field("offer_id", "u64"),
                field("by", "Address"),
                field("refund_amount", "i128"),
            ],
        },
        EventSchema {
            struct_name: "ContractUpgraded".into(),
            fields: vec![
                field("new_wasm_hash", "BytesN<32>"),
                field("new_version", "u32"),
                field("upgraded_at", "u64"),
                field("upgraded_by", "Address"),
            ],
        },
    ]
}

/// Verify that no existing event struct has had a field removed, reordered,
/// or changed type. This enforces the additive-only evolution policy defined
/// in `docs/event-schema-evolution.md`.
///
/// The current registry is compared against the baseline. If the baseline
/// needs updating (because a new field was intentionally appended), update
/// `baseline_event_registry()` to match.
///
/// # Panics
/// - If a field name present in the baseline is absent from the current registry.
/// - If a field's type changed between baseline and current registry.
/// - If a field's ordinal position changed (reordering).
#[test]
fn test_additive_only_evolution() {
    let baseline = baseline_event_registry();
    let current = stream_event_registry();
    let mut errors: Vec<String> = Vec::new();

    for b in &baseline {
        let c = match current.iter().find(|c| c.struct_name == b.struct_name) {
            Some(c) => c,
            None => {
                errors.push(format!(
                    "EVENT '{}' was present in the baseline but is MISSING from the current registry.\n\
                     Events must not be removed. If this is intentional, update baseline_event_registry().",
                    b.struct_name
                ));
                continue;
            }
        };

        // Check each baseline field exists at the same ordinal with the same type.
        for (i, bf) in b.fields.iter().enumerate() {
            if i >= c.fields.len() {
                errors.push(format!(
                    "FIELD '{}' at ordinal {} of struct '{}' (type: {}) was present in the\n\
                     baseline but is MISSING from the current registry. Fields must not be removed.\n\
                     If this is an intentional deprecation, follow the process in\n\
                     docs/event-schema-evolution.md and update the baseline.",
                    bf.name, i, b.struct_name, bf.typ
                ));
                continue;
            }

            let cf = &c.fields[i];

            // Check name match at this ordinal.
            if cf.name != bf.name {
                errors.push(format!(
                    "FIELD REORDERING in struct '{}': at ordinal {} expected field '{}'\n\
                     but found '{}'. Fields must not be reordered.",
                    b.struct_name, i, bf.name, cf.name
                ));
            }

            // Check type match at this ordinal.
            if normalize_type(&cf.typ) != normalize_type(&bf.typ) {
                errors.push(format!(
                    "FIELD TYPE CHANGE in struct '{}': field '{}' changed from '{}'\n\
                     to '{}'. Field types must not change.",
                    b.struct_name, bf.name, bf.typ, cf.typ
                ));
            }
        }

        // Check that new fields are ONLY appended (i.e., the baseline prefix is preserved).
        if c.fields.len() < b.fields.len() {
            // Already reported above as missing fields.
        }
    }

    if !errors.is_empty() {
        let msg = errors.join("\n---\n");
        panic!(
            "\n\n=== ADDITIVE-ONLY EVOLUTION VIOLATIONS ===\n\
             The following changes violate the additive-only event schema evolution\n\
             policy defined in docs/event-schema-evolution.md:\n\n{}\n\n\
             ACTION REQUIRED:\n\
             - If you need to add a field, append it at the END of the struct.\n\
             - If you need to deprecate a field, follow the process in the policy doc.\n\
             - If the baseline itself needs updating, update baseline_event_registry().\n",
            msg
        );
    }
}

/// Verify that every event struct's fields have safe default types for new
/// (appended) fields. This enforces Rule 3 from docs/event-schema-evolution.md:
/// new fields must always be `Option<T>` or have a safe zero-value default.
///
/// This test operates on the current registry. It checks that any field
/// that is NOT present in the baseline (i.e., was appended later) uses a
/// safe default type.
#[test]
fn test_new_fields_have_safe_defaults() {
    let baseline = baseline_event_registry();
    let current = stream_event_registry();
    let mut errors: Vec<String> = Vec::new();

    // Known safe default types
    fn is_safe_default_type(typ: &str) -> bool {
        let normalized = normalize_type(typ);
        normalized.starts_with("Option<")
            || normalized == "bool"
            || normalized == "u64"
            || normalized == "u32"
            || normalized == "i128"
    }

    for c in &current {
        let b = baseline.iter().find(|b| b.struct_name == c.struct_name);
        let baseline_field_count = b.map(|b| b.fields.len()).unwrap_or(0);

        // Check only fields beyond the baseline count (newly appended fields).
        for i in baseline_field_count..c.fields.len() {
            let f = &c.fields[i];
            if !is_safe_default_type(&f.typ) {
                errors.push(format!(
                    "NEW FIELD '{}.{}: {}' (ordinal {}) does not have a safe default type.\n\
                     Per docs/event-schema-evolution.md §4.1, new fields MUST use a safe\n\
                     default type (Option<T>, bool, u64, u32, or i128).\n\
                     Consider wrapping in Option<...> instead.",
                    c.struct_name, f.name, f.typ, i
                ));
            }
        }
    }

    if !errors.is_empty() {
        let msg = errors.join("\n---\n");
        panic!(
            "\n\n=== SAFE DEFAULT TYPE VIOLATIONS ===\n\n{}\n",
            msg
        );
    }
}

/// Verify that every topic symbol defined in docs/events.md matches a known
/// set of permanently reserved topic symbols. This enforces the topic
/// permanence rule from docs/event-schema-evolution.md §3.
///
/// The known topic list is sourced from `events.rs` and `lib.rs` emit calls.
/// Any undocumented topic symbol in the docs is flagged.
///
/// NOTE: This test does NOT parse Rust source to extract topics (that would
/// require a full Rust parser). Instead it relies on the hand-maintained
/// registry of topic symbols below, which must be kept in sync with events.rs.
#[test]
fn test_topic_symbols_are_documented() {
    // Hand-maintained list of every topic symbol in the contract.
    // This MUST be kept in sync with `contracts/stream/src/events.rs`.
    let known_topics: &[&str] = &[
        "created",
        "withdrew",
        "wdraw_to",
        "cancelled",
        "completed",
        "closed",
        "paused",
        "resumed",
        "rate_upd",
        "rate_dec",
        "rate_cap",
        "end_shrt",
        "end_ext",
        "top_up",
        "health",
        "recp_upd",
        "gl_pause",
        "gl_resume",
        "ct_pause",
        "pr_pause",
        "pr_resume",
        "ac_set",
        "ac_revoke",
        "ac_trig",
        "ex_swept",
        "cloned",
        "kp_cncl",
        "decomm",
        "sndr_xfr",
        "renewed",
        "claim_own",
        "del_share",
        "offr_crt",
        "offr_acc",
        "offr_cxl",
        "upgraded",
        "AdminUpd",
        // Reserved topics
        "migrated",
        "tmpl_def",
        "res_rel",
    ];

    let doc_raw = include_str!("../../../docs/events.md");
    let mut missing_from_docs: Vec<&str> = Vec::new();

    for topic in known_topics {
        // Check topic appears as a code-refenced string in docs/events.md.
        // We look for the topic surrounded by quotes or backticks.
        let search_patterns = [
            &format!("\"{}\"", topic),
            &format!("`{}`", topic),
            &format!("[\"{}\"", topic),
        ];
        let found = search_patterns.iter().any(|p| doc_raw.contains(p.as_str()));
        if !found {
            // Also check for the topic in the event table or additional topics list
            // by doing a more lenient search on the topic string alone
            if !doc_raw.contains(topic) {
                missing_from_docs.push(topic);
            }
        }
    }

    if !missing_from_docs.is_empty() {
        panic!(
            "\n\n=== UNDOCUMENTED TOPIC SYMBOLS ===\n\
             The following topic symbols are defined in events.rs but not found\n\
             in docs/events.md. Per the topic permanence policy, every active\n\
             topic symbol must be documented:\n\
             {:?}\n\
             Add these topics to the event table or additional-topics list in\n\
             docs/events.md.",
            missing_from_docs
        );
    }
}

/// Verify that the number of topic elements (cardinality) documented in
/// docs/events.md matches the canonical cardinality defined in events.rs.
///
/// This test uses a hand-maintained cardinality table that MUST be kept in
/// sync with the emit helpers in events.rs.
#[test]
fn test_topic_cardinality_is_fixed() {
    // Hand-maintained cardinality table for every topic symbol.
    // Source: events.rs emit helper signatures.
    // Format: (topic, expected_cardinality)
    let expected_cardinality: &[(&str, usize)] = &[
        ("created", 2),    // [symbol_short!("created"), stream_id]
        ("withdrew", 2),   // [symbol_short!("withdrew"), stream_id]
        ("wdraw_to", 2),   // [symbol_short!("wdraw_to"), stream_id]
        ("cancelled", 2),  // [symbol_short!("cancelled"), stream_id]
        ("completed", 2),  // [symbol_short!("completed"), stream_id]
        ("closed", 2),     // [symbol_short!("closed"), stream_id]
        ("paused", 2),     // [symbol_short!("paused"), stream_id]
        ("resumed", 2),    // [symbol_short!("resumed"), stream_id]
        ("rate_upd", 2),   // [symbol_short!("rate_upd"), stream_id]
        ("rate_dec", 2),   // [symbol_short!("rate_dec"), stream_id]
        ("rate_cap", 2),   // [symbol_short!("rate_cap"), stream_id]
        ("end_shrt", 2),   // [symbol_short!("end_shrt"), stream_id]
        ("end_ext", 2),    // [symbol_short!("end_ext"), stream_id]
        ("top_up", 2),     // [symbol_short!("top_up"), stream_id]
        ("health", 2),     // [symbol_short!("health"), stream_id]
        ("recp_upd", 2),   // [symbol_short!("recp_upd"), stream_id]
        ("gl_pause", 1),   // [symbol_short!("gl_pause")]
        ("gl_resume", 1),  // [symbol_short!("gl_resume")]
        ("ct_pause", 1),   // [symbol_short!("ct_pause")]
        ("pr_pause", 2),   // [symbol_short!("pr_pause"), admin]
        ("pr_resume", 2),  // [symbol_short!("pr_resume"), admin]
        ("ac_set", 2),     // [symbol_short!("ac_set"), stream_id]
        ("ac_revoke", 2),  // [symbol_short!("ac_revoke"), stream_id]
        ("ac_trig", 2),    // [symbol_short!("ac_trig"), stream_id]
        ("ex_swept", 2),   // [symbol_short!("ex_swept"), recipient]
        ("cloned", 2),     // [symbol_short!("cloned"), stream_id]
        ("kp_cncl", 2),    // [symbol_short!("kp_cncl"), stream_id]
        ("decomm", 2),     // [symbol_short!("decomm"), stream_id]
        ("sndr_xfr", 2),   // [symbol_short!("sndr_xfr"), stream_id]
        ("renewed", 3),    // [symbol_short!("renewed"), old_stream_id, new_stream_id]
        ("claim_own", 2),  // [symbol_short!("claim_own"), stream_id]
        ("del_share", 2),  // [symbol_short!("del_share"), stream_id]
        ("offr_crt", 2),   // [symbol_short!("offr_crt"), offer_id]
        ("offr_acc", 2),   // [symbol_short!("offr_acc"), offer_id]
        ("offr_cxl", 2),   // [symbol_short!("offr_cxl"), offer_id]
        ("upgraded", 1),   // [symbol_short!("upgraded")]
        ("AdminUpd", 1),   // [symbol_short!("AdminUpd")]
        ("migrated", 1),   // Reserved
    ];

    let doc_raw = include_str!("../../../docs/events.md");
    let mut errors: Vec<String> = Vec::new();

    for (topic, expected_card) in expected_cardinality {
        // We can't perfectly parse the markdown to extract cardinality, so
        // we do a best-effort check: look for the topic in the event table
        // and verify the topic entry matches expectations.
        //
        // We look for the pattern `| <topic>` or `| "<topic>"` in the table.
        let table_patterns = [
            format!("| `{}` |", topic),
            format!("| `\"{}\"`", topic),
            format!("`{}`", topic),
        ];
        let topic_found = table_patterns.iter().any(|p| doc_raw.contains(p.as_str()))
            || doc_raw.contains(topic);

        if !topic_found {
            errors.push(format!(
                "TOPIC '{}' (expected cardinality {}) not found in docs/events.md.",
                topic, expected_card
            ));
            continue;
        }

        // Verify cardinality from the document: check topic patterns that
        // indicate single vs. multi-topic cardinality.
        //
        // Single topic:   `["AdminUpd"]`           (topic, close bracket)
        // Multi topic:    `["created", stream_id]` (topic, comma separator)
        let single_topic_marker = format!("`\"{}\"]`", topic);   // matches `"AdminUpd"]` at end of topic list
        let multi_topic_marker = format!("\"{}\",", topic);       // matches `"created",` with trailing comma

        let has_single = doc_raw.contains(&single_topic_marker);
        let has_multi = doc_raw.contains(&multi_topic_marker);

        if *expected_card == 1 && has_multi && !has_single {
            errors.push(format!(
                "TOPIC '{}' has expected cardinality 1 but appears to be documented\n\
                 with multiple topic elements (found comma-separated pattern) in docs/events.md.",
                topic
            ));
        }
        if *expected_card >= 2 && !has_multi && has_single {
            errors.push(format!(
                "TOPIC '{}' has expected cardinality {} but appears to be documented\n\
                 with a single topic element (no comma-separated pattern) in docs/events.md.",
                topic, expected_card
            ));
        }
    }

    if !errors.is_empty() {
        let msg = errors.join("\n");
        panic!(
            "\n\n=== TOPIC CARDINALITY ISSUES ===\n\n{}\n\
             Add the missing topics to docs/events.md.",
            msg
        );
    }
}

// ---------------------------------------------------------------------------
// Cross-check logic
// ---------------------------------------------------------------------------

#[test]
fn test_event_schemas_consistent_with_docs() {
    let doc_raw = include_str!("../../../docs/events.md");
    let doc_schemas = parse_doc_schemas(doc_raw);

    // Build lookup: struct_name -> (&name, &[fields])
    let mut doc_map: HashMap<&str, &[EventField]> = HashMap::new();
    for s in &doc_schemas {
        doc_map.insert(s.struct_name.as_str(), s.fields.as_slice());
    }

    // Build reverse lookup: struct_name -> &EventSchema
    let doc_full: HashMap<&str, &EventSchema> = doc_schemas
        .iter()
        .map(|s| (s.struct_name.as_str(), s))
        .collect();

    let registry = stream_event_registry();
    let mut errors: Vec<String> = Vec::new();

    // Forward check: every registry struct must be in docs
    for reg in &registry {
        let doc_schema = doc_full.get(reg.struct_name.as_str());
        match doc_schema {
            None => {
                errors.push(format!(
                    "EVENT '{}' is defined in code (registry) but NOT found in docs/events.md.\n\
                     Add its documented shape to docs/events.md",
                    reg.struct_name
                ));
            }
            Some(doc) => {
                // Check each field in the registry is in the docs
                for rf in &reg.fields {
                    let found = doc.fields.iter().any(|df| {
                        df.name == rf.name && normalize_type(&df.typ) == normalize_type(&rf.typ)
                    });
                    if !found {
                        errors.push(format!(
                            "FIELD '{}.{}: {}' exists in code but NOT in docs/events.md.\n\
                             Documented fields: {:?}",
                            reg.struct_name,
                            rf.name,
                            rf.typ,
                            doc.fields
                                .iter()
                                .map(|f| format!("{}:{}", f.name, f.typ))
                                .collect::<Vec<_>>()
                        ));
                    }
                }
                // Check each field in the docs is in the registry
                for df in &doc.fields {
                    let found = reg.fields.iter().any(|rf| {
                        rf.name == df.name && normalize_type(&rf.typ) == normalize_type(&df.typ)
                    });
                    if !found {
                        errors.push(format!(
                            "FIELD '{}.{}: {}' exists in docs/events.md but NOT in code registry.\n\
                             Registry fields: {:?}",
                            reg.struct_name, df.name, df.typ,
                            reg.fields.iter().map(|f| format!("{}:{}", f.name, f.typ)).collect::<Vec<_>>()
                        ));
                    }
                }
            }
        }
    }

    // Reverse check: every doc struct that is a stream event must be in registry
    for (doc_name, _) in &doc_map {
        let in_registry = registry.iter().any(|r| r.struct_name == *doc_name);
        if !in_registry {
            // Only flag structs that look like stream event payloads
            // (skip things like PauseReason, Stream, StreamOffer, etc.)
            let known_non_events = [
                "PauseReason",
                "PauseRecord",
                "PauseInfo",
                "Stream",
                "StreamOffer",
                "CreateStreamResult",
                "BatchWithdrawResult",
                "WithdrawToParam",
                "AutoClaimValidPayload",
                "AutoClaimInvalidPayload",
                "RotationEntry",
                "PendingRecipientUpdate",
                "StreamHealth",
                "Reservation",
                "Page",
            ];
            if !known_non_events.contains(doc_name) {
                errors.push(format!(
                    "EVENT '{}' is documented in docs/events.md but NOT in code registry.\n\
                     Either it is not a stream event (add to known_non_events list if so),\n\
                     or add it to the registry in the test file.",
                    doc_name
                ));
            }
        }
    }

    if !errors.is_empty() {
        let msg = errors.join("\n---\n");
        panic!(
            "\n\n=== EVENT SCHEMA MISMATCHES FOUND ===\n\
             The following divergences exist between the code event structs\n\
             and their documented shapes in docs/events.md:\n\n{}\n\n\
             ACTION REQUIRED: Update either the code structs or docs/events.md\n\
             to bring them into agreement.\n",
            msg
        );
    }
}
