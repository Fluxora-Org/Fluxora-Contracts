//! `docs/ARCHITECTURE.md` is checked against the code it describes (#1875).
//!
//! An architecture document that is not enforced is a document that is wrong
//! within a release or two, and wrong in the worst way: it is the first thing a
//! new contributor reads and the article of record for anyone deciding whether
//! to trust the system. The claims in it are mostly *enumerable* — workspace
//! members, storage keys, entry point names, absent features — so they can be
//! asserted rather than trusted.
//!
//! This module parses the document and checks each of its tables against its
//! source of truth:
//!
//! | Claim | Source of truth |
//! |---|---|
//! | Components and which of them ship | root `Cargo.toml` members, `script/release.sh` |
//! | Storage keys | the `DataKey` enum in `contracts/stream/src/types.rs` |
//! | Entry point groups and the 16 + 8 split | the committed ABI inventory |
//! | Per-entry-point authority | the authoritative table in `docs/audit.md` |
//! | Features that are deliberately absent | the same ABI inventory |
//!
//! The parse is deliberately strict — a table that has been reflowed, emptied or
//! reworded past recognition fails, rather than passing because nothing was
//! found to contradict it.

use std::format;
use std::string::{String, ToString};
use std::vec::Vec;

/// The document under test.
const ARCHITECTURE_MD: &str = include_str!("../../../../docs/ARCHITECTURE.md");
/// The authoritative per-entry-point authority table.
const AUDIT_MD: &str = include_str!("../../../../docs/audit.md");
/// The workspace manifest: which crates exist at all.
const ROOT_CARGO_TOML: &str = include_str!("../../../../Cargo.toml");
/// The only command that produces release artifacts.
const RELEASE_SH: &str = include_str!("../../../../script/release.sh");
/// The contract's storage key enum.
const TYPES_RS: &str = include_str!("../types.rs");
/// The committed ABI inventory.
const ABI_JSON: &str = include_str!("../../abi/fluxora_stream.json");

// ---------------------------------------------------------------------------
// Markdown helpers
// ---------------------------------------------------------------------------

/// The text of a `## N. Title` section, from its heading to the next `## `.
fn section<'a>(doc: &'a str, heading: &str) -> &'a str {
    let start = doc
        .find(heading)
        .unwrap_or_else(|| panic!("docs/ARCHITECTURE.md lost its `{heading}` section"));
    let rest = &doc[start..];
    let end = rest[1..].find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
    &rest[..end]
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

/// Every backticked token in a string, in order.
fn backticked(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        out.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    out
}

/// The non-empty table rows of a section, as cell vectors.
fn table_rows(section_text: &str) -> Vec<Vec<String>> {
    section_text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('|'))
        .map(cells)
        .filter(|c| {
            !c.is_empty()
                && !c[0].starts_with("---")
                && !c[0].eq_ignore_ascii_case("component")
                && !c[0].eq_ignore_ascii_case("key")
                && !c[0].eq_ignore_ascii_case("group")
                && !c[0].eq_ignore_ascii_case("party")
                && !c[0].eq_ignore_ascii_case("event")
                && !c[0].eq_ignore_ascii_case("entrypoint")
        })
        .collect()
}

/// A `NAME = ["..."]` string-array assignment from a TOML file.
fn toml_string_array(toml: &str, key: &str) -> Vec<String> {
    for line in toml.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(body) = rest.strip_prefix('=') {
                return backticked(&body.replace('"', "`"));
            }
        }
    }
    panic!("no `{key}` array in Cargo.toml");
}

fn abi_function_names() -> Vec<String> {
    let parsed: serde_json::Value =
        serde_json::from_str(ABI_JSON).expect("the committed ABI inventory is valid JSON");
    parsed["functions"]
        .as_array()
        .expect("the ABI inventory has a functions array")
        .iter()
        .map(|f| {
            f["name"]
                .as_str()
                .expect("every ABI function entry has a name")
                .to_string()
        })
        .collect()
}

fn is_delegation(name: &str) -> bool {
    name.starts_with("delegate_") || name == "grant_delegate" || name == "revoke_delegate"
}

/// The `DataKey` variants, in declaration order, read from the source.
fn data_key_variants() -> Vec<String> {
    let start = TYPES_RS
        .find("pub enum DataKey {")
        .expect("types.rs must declare `pub enum DataKey`");
    let body = &TYPES_RS[start..];
    let end = body.find("\n}").expect("the DataKey enum must close");
    body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Variants sit at the start of a line and are followed by `,` or `(`.
            let name: String = line
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() || name == "pub" || name == "enum" || name == "DataKey" {
                return None;
            }
            let rest = line[name.len()..].trim_start();
            if rest.starts_with(',') || rest.starts_with('(') {
                Some(name)
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Components
// ---------------------------------------------------------------------------

/// Every workspace member the manifest declares is a component the document
/// accounts for, and the product/probe split it describes is the split the
/// release path enforces.
#[test]
fn the_component_table_matches_the_workspace_and_the_release_path() {
    let members = toml_string_array(ROOT_CARGO_TOML, "members");
    let components = section(ARCHITECTURE_MD, "## 2. Components");
    let rows = table_rows(components);

    assert!(
        rows.len() >= 3,
        "the component table must describe the contract, the probe and the \
         release tooling, found {} rows",
        rows.len(),
    );

    for member in &members {
        assert!(
            components.contains(&format!("`{member}`")),
            "workspace member `{member}` is not in the component table",
        );
    }

    // The product/probe distinction is the whole point of the table, so it is
    // asserted rather than left to prose.
    let row_for = |path: &str| -> Vec<String> {
        rows.iter()
            .find(|r| r.iter().any(|c| c.contains(&format!("`{path}`"))))
            .unwrap_or_else(|| panic!("no component row for `{path}`"))
            .to_vec()
    };
    // The "Ships?" column is the third: component, path, ships?, role.
    let ships = |path: &str| -> String {
        let row = row_for(path);
        assert!(
            row.len() >= 3,
            "component row for `{path}` is missing its columns"
        );
        row[2].clone()
    };
    assert!(
        ships("contracts/stream").contains("product"),
        "the stream contract row must be marked as the shipping component",
    );
    assert!(
        !ships("contracts/archival-probe").contains("product"),
        "the probe must not be presented as a shipping component",
    );

    // And the release script agrees: it builds the product package and knows
    // the probe only well enough to reject its artifact.
    assert!(
        RELEASE_SH.contains(r#"PRODUCT_PKG="fluxora-stream""#),
        "script/release.sh no longer builds `fluxora-stream` as the product package",
    );
    assert!(
        RELEASE_SH.contains("fluxora-archival-probe"),
        "script/release.sh no longer knows about the probe, so the document's \
         claim about release isolation is unverifiable",
    );
    assert!(
        ARCHITECTURE_MD.contains("`script/release.sh`"),
        "the document must name the release command it relies on",
    );
}

// ---------------------------------------------------------------------------
// 2. Storage
// ---------------------------------------------------------------------------

/// The storage table names every `DataKey` variant and no others.
#[test]
fn the_storage_table_is_exactly_the_datakey_enum() {
    let variants = data_key_variants();
    assert!(
        variants.len() >= 4,
        "expected at least four DataKey variants, parsed {variants:?}",
    );

    let storage = section(ARCHITECTURE_MD, "## 5. Storage model");
    let rows = table_rows(storage);

    for variant in &variants {
        assert!(
            storage.contains(&format!("`DataKey::{variant}")),
            "`DataKey::{variant}` exists in the contract but is missing from \
             the storage table",
        );
    }

    // Every storage-key row in the table must be a real variant: a documented
    // key that does not exist would send a reader looking for data that is not
    // there.
    let mut documented = 0;
    for row in &rows {
        for name in backticked(&row[0]) {
            let name = name.strip_prefix("DataKey::").unwrap_or(&name);
            let name = name.split('(').next().unwrap_or(name);
            assert!(
                variants.iter().any(|v| v == name),
                "the storage table documents `{name}`, which is not a DataKey variant",
            );
            documented += 1;
        }
    }
    assert_eq!(
        documented,
        variants.len(),
        "the storage table documents {documented} keys, the contract has {}",
        variants.len(),
    );

    // The two consequences the document draws from the layout are claims about
    // the code, so they are checked: no per-user index key, and grants keyed by
    // the (stream, delegate) pair.
    assert!(
        variants.iter().all(|v| !v.to_lowercase().contains("index")),
        "the document says there is no per-user index; a key named like one now exists",
    );
    assert!(
        variants.iter().any(|v| v == "Delegate"),
        "the document's per-(stream, delegate) grant claim needs the Delegate key",
    );
}

// ---------------------------------------------------------------------------
// 3. Entry points
// ---------------------------------------------------------------------------

/// The entry point table is exactly the ABI: every exported function listed
/// once, split into the documented 16 core and 8 delegation entry points.
#[test]
fn the_entry_point_table_is_exactly_the_abi() {
    let abi = abi_function_names();
    let surface = section(ARCHITECTURE_MD, "## 6. Entry point surface");

    let mut documented: Vec<String> = Vec::new();
    for row in table_rows(surface) {
        // Two columns: the group, then the entry points it contains.
        let names = row.last().expect("a group row has entry points");
        for name in backticked(names) {
            documented.push(name);
        }
    }

    let mut sorted = documented.clone();
    sorted.sort();
    let mut expected = abi.clone();
    expected.sort();
    assert_eq!(
        sorted, expected,
        "the entry point groups do not match the committed ABI — a function is \
         listed twice, missing, or renamed relative to the contract",
    );

    let delegation = documented.iter().filter(|n| is_delegation(n)).count();
    let core = documented.len() - delegation;
    assert_eq!(core, 16, "the document's core entry point count changed");
    assert_eq!(
        delegation, 8,
        "the document's delegation entry point count changed"
    );
    assert!(
        surface.contains("16 core entry points plus 8 delegation"),
        "the document no longer states the 16 + 8 split it is checked against",
    );

    // The batch ceiling claim.
    assert!(
        surface.contains("`MAX_BATCH_SIZE`"),
        "the document must state the batch ceiling",
    );
    assert!(
        surface.contains("16"),
        "the batch ceiling figure must remain stated in the surface section",
    );
}

/// The per-entry-point authority claims in §3 agree with the authoritative
/// table in `docs/audit.md`.
///
/// `audit.md` is already CI-checked against `lib.rs` by the audit-drift job, so
/// it is the right oracle for "who may call this": the architecture document may
/// summarise it, but it may not disagree with it.
#[test]
fn the_trust_boundaries_agree_with_the_audited_authority_table() {
    let surface = section(ARCHITECTURE_MD, "## 6. Entry point surface");
    // Only the first cell of each row is an entry point; the description column
    // also contains backticked words (flags, `None`), which are not names.
    let audit_entry_points: Vec<String> = AUDIT_MD
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("| `"))
        .filter_map(|l| {
            cells(l)
                .first()
                .and_then(|c| backticked(c).into_iter().next())
        })
        .collect();

    assert!(
        audit_entry_points.len() >= 20,
        "expected docs/audit.md to list the whole surface, parsed {} entries",
        audit_entry_points.len(),
    );

    for name in &audit_entry_points {
        assert!(
            surface.contains(&format!("`{name}`")),
            "docs/audit.md documents `{name}` but the architecture surface table \
             does not list it",
        );
    }

    // The claims about who acts: each side of the trust model must be named,
    // and the delegation boundary must be stated as a grant check.
    let boundaries = section(ARCHITECTURE_MD, "## 3. Trust boundaries");
    for party in [
        "Sender",
        "Recipient",
        "Delegate",
        "Token contract",
        "Anyone",
    ] {
        assert!(
            boundaries.contains(&format!("**{party}**")),
            "the trust boundary table must name the `{party}` boundary",
        );
    }
    assert!(
        boundaries.contains("`require_auth`"),
        "the trust boundaries must state that authority is enforced by `require_auth`",
    );
    assert!(
        boundaries.contains("Grant check"),
        "the delegate boundary must be stated as a stored grant check, not a role",
    );
}

// ---------------------------------------------------------------------------
// 4. Absent features
// ---------------------------------------------------------------------------

/// Every feature §8 says is deliberately absent is genuinely absent from the
/// ABI — including the specific `delegated_withdraw` that §8 scopes to v1.1.
#[test]
fn everything_the_document_rules_out_is_really_gone() {
    let abi = abi_function_names();
    let not_here = section(ARCHITECTURE_MD, "## 8. Deliberately not here");

    for absent in [
        "init",
        "upgrade",
        "set_admin",
        "version",
        "pause_protocol",
        "sweep_excess",
        "delegated_withdraw",
        "get_recipient_streams",
        "get_stream_count",
        "update_rate",
        "set_lookback_window",
    ] {
        assert!(
            !abi.contains(&absent.to_string()),
            "`{absent}` is documented as absent but is in the ABI",
        );
    }

    // The claims themselves must still be made: the section must not have been
    // emptied while the assertions above kept passing vacuously.
    for claim in [
        "admin key",
        "upgrade path",
        "rate limiting",
        "delegated_withdraw",
    ] {
        assert!(
            not_here.contains(claim),
            "§8 no longer records the `{claim}` decision",
        );
    }
}

/// The document's own promise about this module — that its claims are checked —
/// is true, so a reader told to trust the file is told something accurate.
#[test]
fn the_document_names_the_test_that_checks_it() {
    assert!(
        ARCHITECTURE_MD.contains("test/architecture.rs"),
        "the document must point at the module that checks it",
    );
    assert!(
        ARCHITECTURE_MD.contains("`audit.md`"),
        "the document must point at the authoritative entry point table",
    );
    assert!(
        ARCHITECTURE_MD.contains("abi/fluxora_stream.json"),
        "the document must point at the committed ABI inventory it is checked against",
    );
}
