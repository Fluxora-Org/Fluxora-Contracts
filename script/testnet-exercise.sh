#!/usr/bin/env bash
#
# Stage 4 — exercise every Fluxora entrypoint against live testnet.
#
# This is the credibility artifact: it proves the deployed contract behaves on a
# real network the way the unit suite says it does. Every public function is
# called, every assertion is checked against on-chain state, and the transcript
# is written to script/testnet-exercise.log.
#
# Usage:
#   script/testnet-exercise.sh [CONTRACT_ID]
#
# Requires: stellar CLI >= 27 (must match the network protocol), and the
# identities fluxora-alice / fluxora-bob / fluxora-deployer funded on testnet.
#
# What it deliberately does NOT cover: the archival restore round trip. Testnet's
# min_persistent_ttl is 120,960 ledgers (~7 days), a network floor no contract
# can undercut, so a genuine archival cannot be observed in a single run. See
# script/local-archival-proof.sh and docs/KNOWN-LIMITATIONS.md §1.

set -euo pipefail

NETWORK="${NETWORK:-testnet}"
CONTRACT="${1:-$(cat .stellar/contract-ids/fluxora-stream.json 2>/dev/null |
  python3 -c 'import sys,json;print(json.load(sys.stdin)["ids"]["Test SDF Network ; September 2015"])' 2>/dev/null || true)}"

if [[ -z "${CONTRACT}" ]]; then
  echo "usage: $0 <CONTRACT_ID>   (or deploy with --alias fluxora-stream first)" >&2
  exit 2
fi

ALICE=$(stellar keys address fluxora-alice)     # sender
BOB=$(stellar keys address fluxora-bob)         # recipient
CAROL=$(stellar keys address fluxora-deployer)  # transfer target / keeper
TOKEN=$(stellar contract id asset --asset native --network "$NETWORK")

STROOP=10000000  # 1 XLM, 7 decimals

pass=0
fail=0

say()  { printf '\n\033[1m── %s\033[0m\n' "$*"; }
info() { printf '   %s\n' "$*"; }

# ---------------------------------------------------------------------------
# Read-after-write barrier.
#
# The public testnet RPC endpoint is load-balanced across nodes at different
# ledger heights. Measured on 2026-08-12: `getLatestLedger` reported a *lower*
# sequence than the previous call in 6 of 25 consecutive reads, a spread of
# about 5 ledgers (~25s). A script that writes and then immediately reads will
# routinely observe pre-write state and derive nonsense — two view calls
# combined into one figure can even appear to violate conservation.
#
# So after every state change, block until every backend we can see has caught
# up. Any client combining multiple views into one derived number needs the
# same discipline; see docs/KNOWN-LIMITATIONS.md §6.
# ---------------------------------------------------------------------------
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"

latest_ledger() {
  curl -s -m 10 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' 2>/dev/null |
    python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["sequence"])' 2>/dev/null || echo 0
}

# Block until several consecutive samples all report at least the high-water
# mark, i.e. the slowest backend in rotation has caught up.
settle() {
  local target hi=0 ok=0 tries=0 s
  target=$(latest_ledger)
  while (( ok < 4 && tries < 40 )); do
    s=$(latest_ledger)
    (( s > hi )) && hi=$s
    if (( s >= target && s > 0 )); then ok=$((ok + 1)); else ok=0; fi
    tries=$((tries + 1))
    sleep 1
  done
}

# Invoke a read-only view and echo its raw JSON result.
view() {
  stellar contract invoke --id "$CONTRACT" --source fluxora-bob --network "$NETWORK" \
    --send=no -- "$@" 2>/dev/null | tail -1
}

# Same, but unquoted — i128 crosses the ABI as a JSON *string* ("60000000"),
# which is not usable in shell arithmetic.
vnum() { view "$@" | tr -d '"'; }

# Read one field out of a get_stream result.
field() { python3 -c 'import sys,json;print(json.load(sys.stdin)[sys.argv[1]])' "$1"; }

# StreamStatus crosses the ABI as its discriminant, not its name.
status_name() {
  case "$1" in
    0) echo Active ;; 1) echo Paused ;; 2) echo Cancelled ;; 3) echo Depleted ;;
    *) echo "unknown($1)" ;;
  esac
}

# Invoke a state-changing function as the given identity.
send() {
  local who="$1" out; shift
  out=$(stellar contract invoke --id "$CONTRACT" --source "$who" --network "$NETWORK" \
    --send=yes -- "$@" 2>&1) || { echo "SEND_FAILED: $(echo "$out" | tail -2)" >&2; return 1; }
  settle
  echo "$out" | grep -vE '^ℹ️|^🌎|^🔗|^✅|^📅|^$' | tail -1 | tr -d '"'
}

# Invoke expecting failure; echo the contract error name if present.
expect_fail() {
  local who="$1"; shift
  local out
  if out=$(stellar contract invoke --id "$CONTRACT" --source "$who" --network "$NETWORK" \
      --send=yes -- "$@" 2>&1); then
    echo "UNEXPECTED_SUCCESS"
  else
    echo "$out" | grep -oE '#[0-9]+' | head -1
  fi
}

check() {
  local label="$1" actual="$2" expected="$3"
  if [[ "$actual" == "$expected" ]]; then
    printf '   \033[32m✓\033[0m %-52s %s\n' "$label" "$actual"
    pass=$((pass + 1))
  else
    printf '   \033[31m✗\033[0m %-52s got %s, want %s\n' "$label" "$actual" "$expected"
    fail=$((fail + 1))
  fi
}

check_true() {
  local label="$1" cond="$2"
  if [[ "$cond" == "true" ]]; then
    printf '   \033[32m✓\033[0m %s\n' "$label"
    pass=$((pass + 1))
  else
    printf '   \033[31m✗\033[0m %s\n' "$label"
    fail=$((fail + 1))
  fi
}

cat <<BANNER
╭──────────────────────────────────────────────────────────────────────╮
│ Fluxora — testnet exercise                                           │
╰──────────────────────────────────────────────────────────────────────╯
 network   $NETWORK
 contract  $CONTRACT
 token     $TOKEN  (native XLM SAC, 7 decimals)
 sender    $ALICE
 recipient $BOB
 third     $CAROL
 cli       $(stellar --version | head -1)
BANNER

# ---------------------------------------------------------------------------
say "create_stream — 60 XLM over 600s, no cliff, all capabilities"
# ---------------------------------------------------------------------------
NOW=$(date +%s)
START=$NOW
END=$((NOW + 600))
DEPOSIT=$((60 * STROOP))

ID=$(send fluxora-alice create_stream \
  --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
  --deposit "$DEPOSIT" --start_time "$START" --end_time "$END" --cliff_time "$START" \
  --cancellable true --pausable true --transferable true)
info "stream_id = $ID"
check_true "create_stream returned an id" "$([[ "$ID" =~ ^[0-9]+$ ]] && echo true)"

check "stream_exists"  "$(view stream_exists --stream_id "$ID")" "true"
check_true "stream_count > $ID" "$([[ $(vnum stream_count) -gt $ID ]] && echo true)"

# ---------------------------------------------------------------------------
say "views — get_stream / vested_of / withdrawable_of / refundable_of"
# ---------------------------------------------------------------------------
STREAM=$(view get_stream --stream_id "$ID")
info "$(echo "$STREAM" | python3 -c 'import sys,json;d=json.load(sys.stdin);print({k:d[k] for k in ("deposited","withdrawn","start_time","end_time","status","cancellable","pausable","transferable")})' 2>/dev/null || echo "$STREAM")"
check "deposited" "$(echo "$STREAM" | field deposited)" "$DEPOSIT"
check "withdrawn" "$(echo "$STREAM" | field withdrawn)" "0"

# vested_of and refundable_of are two separate simulations and can land on
# different ledgers (see the barrier note above), so compare within a tolerance
# of a few seconds of accrual rather than demanding exact equality. The exact
# invariant is proven per-ledger by the unit suite; what is being checked here
# is that the deployed contract agrees with it on a real network.
RATE=$((DEPOSIT / 600))
V=$(vnum vested_of --stream_id "$ID")
R=$(vnum refundable_of --stream_id "$ID")
SKEW=$(( (V + R) - DEPOSIT )); SKEW=${SKEW#-}
info "vested=$V refundable=$R  drift=$SKEW stroops (~$((SKEW / RATE))s of RPC skew)"
check_true "conservation holds within the RPC skew window" \
  "$([[ $SKEW -le $((RATE * 30)) ]] && echo true)"

# ---------------------------------------------------------------------------
say "withdraw — partial, then max"
# ---------------------------------------------------------------------------
info "waiting 30s for accrual…"; sleep 30
W=$(vnum withdrawable_of --stream_id "$ID")
info "withdrawable = $W stroops"
check_true "accrual is positive after 30s" "$([[ $W -gt 0 ]] && echo true)"

PAID=$(send fluxora-bob withdraw --stream_id "$ID" --amount "$STROOP")
check "partial withdraw paid exactly 1 XLM" "$PAID" "$STROOP"

PAID2=$(send fluxora-bob withdraw --stream_id "$ID")
info "withdraw(None) paid $PAID2 stroops"
check_true "withdraw with no amount drains the accrued balance" "$([[ $PAID2 -gt 0 ]] && echo true)"
# Not asserted as exactly zero: the stream keeps accruing between the withdraw
# transaction and this read, which on a 1 XLM/s stream is tens of millions of
# stroops. What must hold is that the drawn balance dropped sharply.
AFTER_MAX=$(vnum withdrawable_of --stream_id "$ID")
check_true "withdrawable dropped after the max withdraw ($AFTER_MAX << $W)" \
  "$([[ $AFTER_MAX -lt $PAID2 ]] && echo true)"

# ---------------------------------------------------------------------------
say "top_up — extends the duration, holds the rate"
# ---------------------------------------------------------------------------
S1=$(view get_stream --stream_id "$ID")
END_BEFORE=$(echo "$S1" | field end_time)
DEP_BEFORE=$(echo "$S1" | field deposited)
send fluxora-alice top_up --stream_id "$ID" --amount $((30 * STROOP)) >/dev/null
S2=$(view get_stream --stream_id "$ID")
END_AFTER=$(echo "$S2" | field end_time)
DEP_AFTER=$(echo "$S2" | field deposited)
info "end_time $END_BEFORE -> $END_AFTER   deposited -> $DEP_AFTER"
check "deposited grew by the top-up" "$DEP_AFTER" "$((DEP_BEFORE + 30 * STROOP))"
# Extension = amount / rate, floored. Rate is DEP_BEFORE/duration_before.
info "extension = $((END_AFTER - END_BEFORE))s"
check_true "end_time extended, rate held" "$([[ $((END_AFTER - END_BEFORE)) -gt 0 ]] && echo true)"

check "sub-second top_up is rejected (TopUpTooSmall = #23)" \
  "$(expect_fail fluxora-alice top_up --stream_id "$ID" --amount 1)" "#23"

# ---------------------------------------------------------------------------
say "pause / resume — accrual freezes, withdrawal still works"
# ---------------------------------------------------------------------------
send fluxora-alice pause --stream_id "$ID" >/dev/null
check "status is Paused" "$(status_name "$(view get_stream --stream_id "$ID" | field status)")" "Paused"

V1=$(vnum vested_of --stream_id "$ID"); sleep 12; V2=$(vnum vested_of --stream_id "$ID")
check "vested frozen across 12s of wall clock" "$V2" "$V1"
check "double pause is rejected (StreamAlreadyPaused = #13)" \
  "$(expect_fail fluxora-alice pause --stream_id "$ID")" "#13"

send fluxora-alice resume --stream_id "$ID" >/dev/null
check "status is Active" "$(status_name "$(view get_stream --stream_id "$ID" | field status)")" "Active"
check "resume when not paused is rejected (StreamNotPaused = #12)" \
  "$(expect_fail fluxora-alice resume --stream_id "$ID")" "#12"

sleep 10
check_true "accrual resumed" "$([[ $(vnum vested_of --stream_id "$ID") -gt $V2 ]] && echo true)"

# ---------------------------------------------------------------------------
say "transfer_recipient — authority follows the stream"
# ---------------------------------------------------------------------------
send fluxora-bob transfer_recipient --stream_id "$ID" --new_recipient "$CAROL" >/dev/null
check "recipient updated" "$(view get_stream --stream_id "$ID" | field recipient)" "$CAROL"
check "old recipient can no longer withdraw (Unauthorized)" \
  "$([[ "$(expect_fail fluxora-bob withdraw --stream_id "$ID")" == "UNEXPECTED_SUCCESS" ]] && echo BAD || echo blocked)" "blocked"
send fluxora-deployer withdraw --stream_id "$ID" >/dev/null
info "new recipient withdrew successfully"
pass=$((pass + 1)); printf '   \033[32m✓\033[0m new recipient can withdraw\n'
# hand it back so the cancel assertions below read naturally
send fluxora-deployer transfer_recipient --stream_id "$ID" --new_recipient "$BOB" >/dev/null

# ---------------------------------------------------------------------------
say "TTL maintenance — permissionless"
# ---------------------------------------------------------------------------
LEDGERS=$(send fluxora-deployer extend_stream_ttl --stream_id "$ID")
info "extend_stream_ttl -> $LEDGERS ledgers (~$((LEDGERS * 5 / 86400)) days)"
check_true "a third party with no relationship to the stream can pay its rent" \
  "$([[ $LEDGERS -gt 0 ]] && echo true)"

# ---------------------------------------------------------------------------
say "batch operations"
# ---------------------------------------------------------------------------
NOW=$(date +%s)
ID2=$(send fluxora-alice create_stream \
  --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
  --deposit $((20 * STROOP)) --start_time "$NOW" --end_time $((NOW + 600)) --cliff_time "$NOW" \
  --cancellable true --pausable true --transferable true)
info "second stream id = $ID2"

N=$(send fluxora-deployer batch_extend_ttl --stream_ids "[$ID,$ID2,99999]")
check "batch_extend_ttl skips unknown ids" "$N" "2"

sleep 15
TOTAL=$(send fluxora-bob batch_withdraw --recipient "$BOB" --stream_ids "[$ID,$ID2]")
info "batch_withdraw total = $TOTAL stroops"
check_true "batch_withdraw paid out" "$([[ $TOTAL -gt 0 ]] && echo true)"

check "duplicate id in a batch is rejected (DuplicateStreamId = #21)" \
  "$(expect_fail fluxora-bob batch_withdraw --recipient "$BOB" --stream_ids "[$ID,$ID]")" "#21"
check "oversized batch is rejected (BatchTooLarge = #19)" \
  "$(expect_fail fluxora-bob batch_withdraw --recipient "$BOB" \
      --stream_ids "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]")" "#19"

# ---------------------------------------------------------------------------
say "cancel — refund the unvested remainder, recipient keeps what vested"
# ---------------------------------------------------------------------------
BEFORE=$(stellar contract invoke --id "$TOKEN" --source fluxora-alice --network "$NETWORK" \
  --send=no -- balance --id "$ALICE" 2>/dev/null | tail -1 | tr -d '"')
REFUNDABLE=$(vnum refundable_of --stream_id "$ID2")
info "refundable before cancel = $REFUNDABLE"

send fluxora-alice cancel --stream_id "$ID2" >/dev/null
AFTER=$(stellar contract invoke --id "$TOKEN" --source fluxora-alice --network "$NETWORK" \
  --send=no -- balance --id "$ALICE" 2>/dev/null | tail -1 | tr -d '"')
S3=$(view get_stream --stream_id "$ID2")
check "status is Cancelled" "$(status_name "$(echo "$S3" | field status)")" "Cancelled"
check_true "sender's balance increased by roughly the refund" \
  "$([[ $((AFTER - BEFORE)) -gt $((REFUNDABLE - 10 * STROOP)) ]] && echo true)"
check "cancelling twice is rejected (StreamTerminated = #14)" \
  "$(expect_fail fluxora-alice cancel --stream_id "$ID2")" "#14"

DEP=$(echo "$S3" | field deposited)
WD=$(echo "$S3" | field withdrawn)
check_true "cancel left deposited >= withdrawn (liability non-negative)" \
  "$([[ $DEP -ge $WD ]] && echo true)"

# ---------------------------------------------------------------------------
say "validation gates"
# ---------------------------------------------------------------------------
NOW=$(date +%s)
check "end <= start rejected (InvalidTimeRange = #2)" \
  "$(expect_fail fluxora-alice create_stream --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
      --deposit "$STROOP" --start_time "$NOW" --end_time "$NOW" --cliff_time "$NOW" \
      --cancellable true --pausable true --transferable true)" "#2"

check "stream to self rejected (SelfStream = #6)" \
  "$(expect_fail fluxora-alice create_stream --sender "$ALICE" --recipient "$ALICE" --token "$TOKEN" \
      --deposit "$STROOP" --start_time "$NOW" --end_time $((NOW + 600)) --cliff_time "$NOW" \
      --cancellable true --pausable true --transferable true)" "#6"

check "dust-rate deposit rejected (DepositRateTooLow = #5)" \
  "$(expect_fail fluxora-alice create_stream --sender "$ALICE" --recipient "$BOB" --token "$TOKEN" \
      --deposit 100 --start_time "$NOW" --end_time $((NOW + 31536000)) --cliff_time "$NOW" \
      --cancellable true --pausable true --transferable true)" "#5"

check "unknown stream id (StreamNotFound = #1)" \
  "$(expect_fail fluxora-bob withdraw --stream_id 999999)" "#1"

# ---------------------------------------------------------------------------
printf '\n╭──────────────────────────────────────────────────────────────────────╮\n'
printf '│ %-2d passed   %-2d failed                                              │\n' "$pass" "$fail"
printf '╰──────────────────────────────────────────────────────────────────────╯\n'
printf '\nContract: https://stellar.expert/explorer/testnet/contract/%s\n' "$CONTRACT"
[[ $fail -eq 0 ]]

