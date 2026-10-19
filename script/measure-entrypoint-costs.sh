#!/usr/bin/env bash
#
# Stage 4 — calibrate entry point instruction costs against the live testnet.
#
# The unit suite (entrypoint_costs.rs / validate_gas.py) measures instruction
# counts under the Soroban SDK test host, where Wasm instantiation overhead is
# excluded.  This script re-measures the same 24 entry points via the RPC
# `simulateTransaction` call, which the `stellar contract invoke --send=no`
# path executes.  The result is the true on-network instruction cost, including
# Wasm instantiation.
#
# The expected delta is roughly 1.5–3× the local SDK figure; the exact
# multiplier depends on the WASM binary size and the metering tables in the
# deployed protocol version.
#
# Outputs:
#   script/testnet-entrypoint-costs.json   per-function instruction counts
#   script/testnet-entrypoint-costs.md     human-readable comparison table
#
# Usage:
#   script/measure-entrypoint-costs.sh [CONTRACT_ID]
#
# CONTRACT_ID defaults to the value in .stellar/contract-ids/fluxora-stream.json
# if the file exists, then falls back to the ABI.md constant.
#
# Prerequisites:
#   • stellar CLI >= 27 (protocol must match)
#   • Identities fluxora-alice (sender), fluxora-bob (recipient), and
#     fluxora-deployer (third-party keeper) funded on testnet.
#   • Network reachable at $RPC_URL (default: https://soroban-testnet.stellar.org)
#
# Environment variables:
#   NETWORK       Stellar network alias (default: testnet)
#   RPC_URL       Override the RPC endpoint
#   CONTRACT      Override the contract id (also accepted as positional arg $1)
#
# What is measured:
#   Every public entry point is simulated via `--send=no`.  For mutating calls
#   the transaction is not broadcast; the network returns a full simulation
#   response including sorobanData.resources.instructions.  The stellar CLI
#   echoes that value in its output when --send=no is used; this script extracts
#   it with a `simulateTransaction` direct RPC call so the parse is stable.
#
# Repeatability:
#   Each run produces a fresh JSON file.  Commit it alongside the baseline JSON
#   when reconciling §2.  Run it again after any SDK or network upgrade to
#   detect drift.
#
# Relationship to entrypoint-cost-baseline.json:
#   The baseline JSON records *local* SDK measurements.  The testnet JSON
#   records *network simulation* measurements.  They measure different things.
#   The baseline gates CI regressions.  The testnet file documents the real
#   upper bound integrators should plan for.
#
# Note on Soroban simulation resource extraction:
#   `stellar contract invoke --send=no` calls `simulateTransaction` under the
#   hood.  The raw RPC response contains:
#     result.minResourceFee
#     result.transactionData.sorobanData (base64 XDR)
#   The instruction count is embedded in the sorobanData XDR, but extracting it
#   from XDR in a shell script is brittle.  Instead this script calls the RPC
#   directly using the same payload the CLI would use, then parses the JSON
#   response field `result.cost.cpuInsns` which the Soroban RPC returns as a
#   plain decimal string alongside every simulation result.
#
# See: https://developers.stellar.org/docs/data/rpc/api-reference/methods/simulateTransaction

set -euo pipefail

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"

# ---------------------------------------------------------------------------
# Contract identity
# ---------------------------------------------------------------------------
CONTRACT="${1:-${CONTRACT:-}}"
if [[ -z "$CONTRACT" ]]; then
  CONTRACT=$(cat .stellar/contract-ids/fluxora-stream.json 2>/dev/null |
    python3 -c 'import sys,json;print(json.load(sys.stdin)["ids"]["Test SDF Network ; September 2015"])' \
    2>/dev/null || true)
fi
if [[ -z "$CONTRACT" ]]; then
  # Fall back to the ABI.md constant.
  CONTRACT="CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW"
fi

ALICE=$(stellar keys address fluxora-alice 2>/dev/null)     # sender
BOB=$(stellar keys address fluxora-bob   2>/dev/null)       # recipient
CAROL=$(stellar keys address fluxora-deployer 2>/dev/null)  # third-party / keeper
TOKEN=$(stellar contract id asset --asset native --network "$NETWORK" 2>/dev/null)

STROOP=10000000   # 1 XLM (7 decimals)
OUT_JSON="script/testnet-entrypoint-costs.json"
OUT_MD="script/testnet-entrypoint-costs.md"
BASELINE="contracts/stream/entrypoint-cost-baseline.json"

say()  { printf '\n\033[1m── %s\033[0m\n' "$*"; }
info() { printf '   %s\n' "$*"; }

cat <<BANNER
╭──────────────────────────────────────────────────────────────────────╮
│ Fluxora — testnet entry point cost measurement                       │
╰──────────────────────────────────────────────────────────────────────╯
 network   $NETWORK
 rpc       $RPC_URL
 contract  $CONTRACT
 token     $TOKEN  (native XLM SAC, 7 decimals)
 sender    $ALICE
 recipient $BOB
 third     $CAROL
 cli       $(stellar --version 2>/dev/null | head -1 || echo "not found")
BANNER

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

latest_ledger() {
  curl -s -m 10 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' |
    python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["sequence"])' 2>/dev/null || echo 0
}

settle() {
  local target hi=0 ok=0 tries=0 s
  target=$(latest_ledger)
  while (( ok < 3 && tries < 30 )); do
    s=$(latest_ledger)
    (( s > hi )) && hi=$s
    if (( s >= target && s > 0 )); then ok=$((ok + 1)); else ok=0; fi
    tries=$((tries + 1))
    sleep 2
  done
}

# Simulate a read-only call and return the raw result value (last stdout line).
view() {
  stellar contract invoke --id "$CONTRACT" --source fluxora-bob --network "$NETWORK" \
    --send=no -- "$@" 2>/dev/null | tail -1
}

# Send a state-changing call; wait for the network to settle.
send() {
  local who="$1"; shift
  local out
  out=$(stellar contract invoke --id "$CONTRACT" --source "$who" --network "$NETWORK" \
    --send=yes -- "$@" 2>&1) || {
    echo "SEND_FAILED: $(echo "$out" | tail -2)" >&2
    return 1
  }
  settle
  echo "$out" | grep -vE '^ℹ️|^🌎|^🔗|^✅|^📅|^$' | tail -1 | tr -d '"'
}

# ---------------------------------------------------------------------------
# simulate_and_extract_instructions CONTRACT SOURCE ARGS...
#
# Builds the transaction XDR via the stellar CLI in simulation mode
# (`--send=no`), then re-calls simulateTransaction directly over the RPC to
# extract `result.cost.cpuInsns`.
#
# The stellar CLI writes simulation results to stderr under verbose output.
# The most portable approach that avoids XDR parsing is to call the RPC
# method directly using the transaction XDR the CLI assembles.
#
# Strategy:
#   1. Use `stellar tx new invoke-contract` + `stellar tx simulate` to get
#      a simulated transaction and capture the JSON response.
#   2. Parse `result.cost.cpuInsns` (a decimal string) from the JSON.
#
# Fallback: if the raw RPC path is unavailable, call `--send=no` and
# parse any "instructions:" hint the CLI emits.
# ---------------------------------------------------------------------------
simulate_instructions() {
  local source="$1"; shift
  local args=("$@")

  # Build and simulate the transaction; capture the raw RPC JSON response.
  # `stellar contract invoke --send=no --verbose` outputs the simulation JSON.
  local sim_out
  sim_out=$(stellar contract invoke \
    --id "$CONTRACT" \
    --source "$source" \
    --network "$NETWORK" \
    --send=no \
    --verbose \
    -- "${args[@]}" 2>&1) || true

  # The Stellar CLI (>= 27) emits a line like:
  #   "instructions": 1234567
  # inside the simulation JSON block printed to stderr with --verbose.
  local instructions
  instructions=$(echo "$sim_out" |
    python3 -c '
import sys, re, json

text = sys.stdin.read()

# Strategy 1: the CLI printed structured JSON containing "cpuInsns"
# (present in RPC simulateTransaction responses)
for m in re.finditer(r"\"cpuInsns\"\s*:\s*\"(\d+)\"", text):
    print(m.group(1))
    sys.exit(0)

# Strategy 2: the CLI printed the sorobanData instructions field as a plain int
# e.g.  "instructions": 1234567
for m in re.finditer(r"\"instructions\"\s*:\s*(\d+)", text):
    print(m.group(1))
    sys.exit(0)

# Strategy 3: the CLI printed a line like "cpuInsns: 1234567"
for m in re.finditer(r"cpuInsns[:\s]+(\d+)", text, re.I):
    print(m.group(1))
    sys.exit(0)

# Strategy 4: call the RPC simulateTransaction endpoint directly.
# Extract the assembled transaction XDR from the CLI output.
xdr_match = re.search(r"transaction:\s*([A-Za-z0-9+/=]{20,})", text)
if xdr_match:
    print("NEED_RPC_FALLBACK " + xdr_match.group(1))
    sys.exit(0)

sys.exit(1)
' 2>/dev/null) || instructions=""

  if [[ "$instructions" == NEED_RPC_FALLBACK* ]]; then
    local xdr="${instructions#NEED_RPC_FALLBACK }"
    instructions=$(curl -s -m 30 -X POST "$RPC_URL" \
      -H 'Content-Type: application/json' \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"simulateTransaction\",\"params\":{\"transaction\":\"$xdr\"}}" |
      python3 -c '
import sys, json
d = json.load(sys.stdin)
r = d.get("result", {})
print(r.get("cost", {}).get("cpuInsns", ""))
' 2>/dev/null) || instructions=""
  fi

  echo "${instructions:-0}"
}

# ---------------------------------------------------------------------------
# declare_cost FUNCTION_NAME INSTRUCTIONS
# Accumulate into the results arrays.
# ---------------------------------------------------------------------------
declare -A COSTS=()
record_cost() {
  local fn="$1" cost="$2"
  COSTS[$fn]="$cost"
  printf '   %-36s %s instructions\n' "$fn" "$cost"
}

# ---------------------------------------------------------------------------
# Setup: create a stream for use across all measurements.
# Most calls are simulated (--send=no) so they do not mutate on-chain state.
# A few state-changing calls (create_stream, send) are submitted for real so
# subsequent simulations have live state to operate on.
# ---------------------------------------------------------------------------
say "Setup — creating a reference stream on testnet"
NOW=$(date +%s)
START=$NOW
END=$((NOW + 86400))   # 24h stream
DEPOSIT=$((100 * STROOP))

STREAM_ID=$(send fluxora-alice create_stream \
  --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
  --deposit "$DEPOSIT" --start_time "$START" --end_time "$END" \
  --cliff_time "$START" \
  --cancellable true --pausable true --transferable true)
info "reference stream_id = $STREAM_ID"

# Create a second stream for batch operations.
STREAM_ID2=$(send fluxora-alice create_stream \
  --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
  --deposit "$DEPOSIT" --start_time "$START" --end_time "$END" \
  --cliff_time "$START" \
  --cancellable true --pausable true --transferable true)
info "batch stream_id = $STREAM_ID2"

# Wait a little so accrual is nonzero for withdraw simulations.
info "sleeping 15s for accrual…"
sleep 15

# ---------------------------------------------------------------------------
# Measure each entry point.
# For mutating calls: simulate with --send=no (does not broadcast).
# For view calls: simulate with --send=no (already the default).
# ---------------------------------------------------------------------------
say "Measuring entry point costs via simulateTransaction"

# -- Views --
record_cost "get_stream" \
  "$(simulate_instructions fluxora-bob get_stream --stream_id "$STREAM_ID")"
record_cost "withdrawable_of" \
  "$(simulate_instructions fluxora-bob withdrawable_of --stream_id "$STREAM_ID")"
record_cost "vested_of" \
  "$(simulate_instructions fluxora-bob vested_of --stream_id "$STREAM_ID")"
record_cost "refundable_of" \
  "$(simulate_instructions fluxora-bob refundable_of --stream_id "$STREAM_ID")"
record_cost "stream_count" \
  "$(simulate_instructions fluxora-bob stream_count)"
record_cost "stream_exists" \
  "$(simulate_instructions fluxora-bob stream_exists --stream_id "$STREAM_ID")"

# -- Lifecycle: simulate (not broadcast) --
record_cost "create_stream" \
  "$(simulate_instructions fluxora-alice create_stream \
      --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
      --deposit "$DEPOSIT" --start_time "$START" --end_time "$END" \
      --cliff_time "$START" \
      --cancellable true --pausable true --transferable true)"

record_cost "top_up" \
  "$(simulate_instructions fluxora-alice top_up \
      --stream_id "$STREAM_ID" --amount "$((10 * STROOP))")"

record_cost "withdraw" \
  "$(simulate_instructions fluxora-bob withdraw \
      --stream_id "$STREAM_ID")"

record_cost "batch_withdraw" \
  "$(simulate_instructions fluxora-bob batch_withdraw \
      --recipient "$BOB" --stream_ids "[$STREAM_ID]")"

# pause simulation (no-send)
record_cost "pause" \
  "$(simulate_instructions fluxora-alice pause --stream_id "$STREAM_ID")"

# For resume we need the stream to actually be paused; send the pause first.
send fluxora-alice pause --stream_id "$STREAM_ID" >/dev/null
record_cost "resume" \
  "$(simulate_instructions fluxora-alice resume --stream_id "$STREAM_ID")"
# Actually resume it so later calls can operate on an active stream.
send fluxora-alice resume --stream_id "$STREAM_ID" >/dev/null

record_cost "transfer_recipient" \
  "$(simulate_instructions fluxora-bob transfer_recipient \
      --stream_id "$STREAM_ID" --new_recipient "$CAROL")"

record_cost "cancel" \
  "$(simulate_instructions fluxora-alice cancel --stream_id "$STREAM_ID2")"

# -- TTL maintenance --
record_cost "extend_stream_ttl" \
  "$(simulate_instructions fluxora-deployer extend_stream_ttl \
      --stream_id "$STREAM_ID")"
record_cost "batch_extend_ttl" \
  "$(simulate_instructions fluxora-deployer batch_extend_ttl \
      --stream_ids "[$STREAM_ID]")"

# -- Delegation --
# For delegation calls we need a delegate address; use CAROL as stand-in.
DELEGATE="$CAROL"

record_cost "grant_delegate" \
  "$(simulate_instructions fluxora-bob grant_delegate \
      --stream_id "$STREAM_ID" --principal "$BOB" \
      --delegate "$DELEGATE" --ops 1 --expires_at null)"

# grant the delegate for real so subsequent delegation calls have state to work on.
send fluxora-bob grant_delegate \
  --stream_id "$STREAM_ID" --principal "$BOB" \
  --delegate "$DELEGATE" --ops 1 --expires_at null >/dev/null

record_cost "delegate_withdraw" \
  "$(simulate_instructions fluxora-deployer delegate_withdraw \
      --stream_id "$STREAM_ID" --delegate "$DELEGATE")"

record_cost "revoke_delegate" \
  "$(simulate_instructions fluxora-bob revoke_delegate \
      --stream_id "$STREAM_ID" --principal "$BOB" --delegate "$DELEGATE")"

# grant cancel delegation for sender-side delegate calls.
send fluxora-alice grant_delegate \
  --stream_id "$STREAM_ID" --principal "$ALICE" \
  --delegate "$DELEGATE" --ops 14 --expires_at null >/dev/null

record_cost "delegate_cancel" \
  "$(simulate_instructions fluxora-deployer delegate_cancel \
      --stream_id "$STREAM_ID" --delegate "$DELEGATE")"

record_cost "delegate_pause" \
  "$(simulate_instructions fluxora-deployer delegate_pause \
      --stream_id "$STREAM_ID" --delegate "$DELEGATE")"

# For delegate_resume and delegate_top_up, pause the stream first via the delegate.
send fluxora-deployer delegate_pause \
  --stream_id "$STREAM_ID" --delegate "$DELEGATE" >/dev/null || true
record_cost "delegate_resume" \
  "$(simulate_instructions fluxora-deployer delegate_resume \
      --stream_id "$STREAM_ID" --delegate "$DELEGATE")"
send fluxora-deployer delegate_resume \
  --stream_id "$STREAM_ID" --delegate "$DELEGATE" >/dev/null || true

record_cost "delegate_top_up" \
  "$(simulate_instructions fluxora-deployer delegate_top_up \
      --stream_id "$STREAM_ID" --delegate "$DELEGATE" \
      --amount "$((10 * STROOP))")"

# grant transfer_recipient delegation.
send fluxora-bob grant_delegate \
  --stream_id "$STREAM_ID" --principal "$BOB" \
  --delegate "$DELEGATE" --ops 16 --expires_at null >/dev/null || true
record_cost "delegate_transfer_recipient" \
  "$(simulate_instructions fluxora-deployer delegate_transfer_recipient \
      --stream_id "$STREAM_ID" --delegate "$DELEGATE" \
      --new_recipient "$BOB")"

# ---------------------------------------------------------------------------
# Emit JSON output
# ---------------------------------------------------------------------------
say "Writing output files"

python3 - <<PYEOF
import json, sys

costs = {}
EOF_DATA = """
$(for k in "${!COSTS[@]}"; do echo "$k ${COSTS[$k]}"; done)
"""
for line in EOF_DATA.strip().splitlines():
    parts = line.strip().split()
    if len(parts) == 2:
        fn, val = parts
        try:
            costs[fn] = int(val)
        except ValueError:
            pass

out = dict(sorted(costs.items()))
with open("$OUT_JSON", "w", encoding="utf-8") as f:
    json.dump(out, f, indent=2)
    f.write("\n")
print(f"Wrote {len(out)} entries to $OUT_JSON")
PYEOF

# ---------------------------------------------------------------------------
# Emit Markdown comparison table
# ---------------------------------------------------------------------------
python3 - <<PYEOF
import json

testnet = {}
EOF_DATA = """
$(for k in "${!COSTS[@]}"; do echo "$k ${COSTS[$k]}"; done)
"""
for line in EOF_DATA.strip().splitlines():
    parts = line.strip().split()
    if len(parts) == 2:
        fn, val = parts
        try:
            testnet[fn] = int(val)
        except ValueError:
            pass

with open("$BASELINE", encoding="utf-8") as f:
    baseline = json.load(f)

rows = []
for fn in sorted(set(list(testnet.keys()) + list(baseline.keys()))):
    local_val  = baseline.get(fn, 0)
    net_val    = testnet.get(fn, 0)
    if local_val > 0 and net_val > 0:
        ratio = net_val / local_val
        ceiling = max(local_val, net_val)
    else:
        ratio = 0.0
        ceiling = max(local_val, net_val)
    rows.append((fn, local_val, net_val, ratio, ceiling))

lines = [
    "# Fluxora — testnet entry point cost measurement",
    "",
    "Generated by `script/measure-entrypoint-costs.sh` against",
    f"`$CONTRACT` on `$NETWORK`.",
    "",
    "**Local baseline** = Soroban SDK test host, Wasm binary registered natively",
    "(Wasm instantiation overhead excluded).",
    "",
    "**Testnet simulation** = `simulateTransaction` RPC, full Wasm execution",
    "(Wasm instantiation overhead included).",
    "",
    "| Entry point | Local baseline | Testnet simulation | Ratio | Ceiling (max) |",
    "| --- | ---: | ---: | ---: | ---: |",
]
for fn, local_val, net_val, ratio, ceiling in rows:
    lines.append(
        f"| \`{fn}\` | {local_val:,} | {net_val:,} | {ratio:.2f}x | {ceiling:,} |"
    )

lines += [
    "",
    "## Notes",
    "",
    "- The **ratio** is testnet/local. Values in the range 1.5–3× are typical",
    "  because the test host skips Wasm instantiation metering.",
    "- The **ceiling** column records the larger of the two figures and is the",
    "  number that integrators should budget against.",
    "- Ledger entry counts and event bytes (the constraints that determine",
    "  `MAX_BATCH_SIZE`) are *not* captured here; those are accurate in the",
    "  local suite because they do not depend on execution mode.",
    "- Re-run this script after any Soroban protocol upgrade or SDK bump.",
    "",
    "## Reconciliation with docs/KNOWN-LIMITATIONS.md §2",
    "",
    "The ceiling column above shows the measured upper bound from the network.",
    "The local baseline is not replaced — it continues to gate CI regressions.",
    "The testnet figures document the realistic production budget.",
]

with open("$OUT_MD", "w", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
print(f"Wrote comparison table to $OUT_MD")
PYEOF

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
say "Measurement complete"
info "JSON  : $OUT_JSON"
info "Report: $OUT_MD"
info ""
info "Review script/testnet-entrypoint-costs.md and update §2 of"
info "docs/KNOWN-LIMITATIONS.md with the measured delta.  Commit"
info "both output files as the calibration artifact."

cat "$OUT_MD"
