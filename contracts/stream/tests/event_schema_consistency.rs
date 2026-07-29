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
