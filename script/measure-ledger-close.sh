#!/usr/bin/env bash
#
# Observed ledger close time — the measurement behind
# docs/ledger-close-time.md and the pinned values in storage.rs
# (SECONDS_PER_LEDGER, TTL_SAFETY_MARGIN_PERCENT). The measurement half of
# docs/KNOWN-LIMITATIONS.md §5 ("ledger close time is assumed, not measured",
# narrowed by #1806).
#
# The window is *sustained*, not instantaneous: the mean is taken over every
# ledger the RPC node still serves — testnet retained ~121,000 ledgers
# (≈6.9 days) when this was written — so single-ledger jitter cannot produce a
# verdict. One snapshot gap is meaningless anyway: networks enforce a 5 s
# floor on individual closes while the *average* is what TTL math consumes.
# Per-ledger variance comes from sampled pages.
#
# Usage:
#   script/measure-ledger-close.sh [--verify]
#
#   (no flag)   measure and print a verdict
#   --verify    additionally fail (exit 1) if the margin does not cover the
#               observed mean — the re-check a release runbook can call
#
# Exit codes:
#   0  measurement ran (verdict is printed; may be COVERED or EXPOSED)
#   1  --verify was passed and the margin does not cover the observation
#   2  could not reach the RPC or the window was too small to judge
#
# Environment:
#   RPC_URL          Soroban RPC endpoint (default: Stellar testnet)
#   MEASURE_LEDGERS  minimum window, in ledgers, for a verdict to be
#                    trustworthy (default: 10,000 ≈ 14 h at 5 s)
#   SAMPLE_PAGES     how many 50-ledger pages to sample for per-ledger
#                    variance (default: 24)

set -euo pipefail

RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
MEASURE_LEDGERS="${MEASURE_LEDGERS:-10000}"
SAMPLE_PAGES="${SAMPLE_PAGES:-24}"
PAGE_SIZE=50

VERIFY=false
[[ "${1:-}" == "--verify" ]] && VERIFY=true

# The constants under test. Parsed from the source rather than duplicated here
# so the measurement fails loudly if the constant is renamed or removed.
STREAM_SRC="contracts/stream/src/storage.rs"
ASSUMED=$(sed -n 's/^pub const SECONDS_PER_LEDGER: u64 = \([0-9][0-9]*\);/\1/p' "$STREAM_SRC")
MARGIN=$(sed -n 's/^pub const TTL_SAFETY_MARGIN_PERCENT: u64 = \([0-9][0-9]*\);/\1/p' "$STREAM_SRC")
if [[ -z "$ASSUMED" || -z "$MARGIN" ]]; then
  echo "could not parse SECONDS_PER_LEDGER / TTL_SAFETY_MARGIN_PERCENT from $STREAM_SRC" >&2
  exit 2
fi

say() { printf '\n\033[1m── %s\033[0m\n' "$*"; }

rpc() { # $1 = JSON-RPC payload on stdin, printed to stdout
  curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' -d "$1"
}

fetch_page() { # $1 = start ledger, $2 = limit -> getLedgers response
  rpc "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getLedgers\",\"params\":{\"startLedger\":$1,\"pagination\":{\"limit\":$2}}}"
}

# ---------------------------------------------------------------------------
say "1. discovering the ledger window"
# ---------------------------------------------------------------------------

# Two cheap requests pin the *exact* mean over the whole retention window:
# getLatestLedger gives the newest sequence and its close time, and any
# getLedgers response reports `oldestLedger` / `oldestLedgerCloseTime` — the
# oldest ledger the node still serves — at top level. So
#
#   mean = (latest close - oldest close) / (latest seq - oldest seq)
#
# covers every consecutive ledger the node still has — no sampling error in
# the mean, and no need to download ~2,400 nine-megabyte pages of ledger
# metadata to compute it.
# Responses are written to a temp file and parsed from there, never echoed
# into a pipe: SIGPIPE from the consumer can silently kill the parse.
TMP=$(mktemp)
trap 'rm -f "$TMP" "$TMP.page"' EXIT

rpc_latest() {
  rpc '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' >"$TMP"
}
fetch_page() { # $1 = start ledger, $2 = limit -> writes getLedgers to TMP.page
  curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getLedgers\",\"params\":{\"startLedger\":$1,\"pagination\":{\"limit\":$2}}}" \
    >"$TMP.page"
}

rpc_latest
LATEST_INFO=$(python3 -c '
import sys, json
r = json.load(open(sys.argv[1]))["result"]
print(r["sequence"], int(r["closeTime"]))' "$TMP" 2>/dev/null || true)
if [[ -z "$LATEST_INFO" ]]; then
  echo "could not reach $RPC_URL getLatestLedger:" >&2
  head -c 400 "$TMP" >&2
  echo >&2
  exit 2
fi
read -r LATEST LT <<<"$LATEST_INFO"
fetch_page "$LATEST" 1
OLDEST_INFO=$(python3 -c '
import sys, json
r = json.load(open(sys.argv[1]))["result"]
print(r["oldestLedger"], int(r["oldestLedgerCloseTime"]))' "$TMP.page" 2>/dev/null || true)
if [[ -z "$OLDEST_INFO" ]]; then
  echo "could not read the retention window from $RPC_URL getLedgers:" >&2
  head -c 400 "$TMP.page" >&2
  echo >&2
  exit 2
fi
read -r OLDEST OT <<<"$OLDEST_INFO"
SPAN=$((LATEST - OLDEST))
WINDOW=$((SPAN + 1))
MEAN=$(python3 -c "print(f'{($LT - $OT) / $SPAN:.4f}')")
echo "   latest ledger:   $LATEST  (closed at $LT)"
echo "   oldest served:   $OLDEST  (closed at $OT)"
echo "   window:          $WINDOW ledgers"

if (( WINDOW < MEASURE_LEDGERS )); then
  echo "   window below the $MEASURE_LEDGERS-ledger trust threshold." >&2
  echo "   Set RPC_URL to a fuller node or lower MEASURE_LEDGERS." >&2
  exit 2
fi

# ---------------------------------------------------------------------------
say "2. per-ledger variance across $SAMPLE_PAGES sampled pages"
# ---------------------------------------------------------------------------

# The mean is exact; this pass is about the *distribution* — how much a single
# ledger can deviate, and where the 95th percentile sits. Pages are spread
# evenly across the window and gaps are taken strictly inside each page so no
# boundary gap is counted twice.
: >"$TMP"
i=0
while (( i < SAMPLE_PAGES )); do
  START=$((OLDEST + i * SPAN / SAMPLE_PAGES))
  OK=false
  for ATTEMPT in 1 2 3; do
    fetch_page "$START" "$PAGE_SIZE"
    if python3 -c 'import sys, json; json.load(open(sys.argv[1]))["result"]["ledgers"]' "$TMP.page" 2>/dev/null; then
      OK=true
      break
    fi
    sleep 2  # transient RPC error — back off and retry the same page
  done
  if ! $OK; then
    echo "page at ledger $START kept failing after 3 attempts:" >&2
    head -c 300 "$TMP.page" >&2
    echo >&2
    exit 2
  fi
  python3 -c '
import sys, json
ledgers = json.load(open(sys.argv[1]))["result"]["ledgers"]
for a, b in zip(ledgers, ledgers[1:]):
    if b["sequence"] == a["sequence"] + 1:
        print(int(b["ledgerCloseTime"]) - int(a["ledgerCloseTime"]))' "$TMP.page" >>"$TMP"
  i=$((i + 1))
done
SAMPLED=$(wc -l <"$TMP")
if (( SAMPLED < SAMPLE_PAGES )); then
  echo "only $SAMPLED usable gaps sampled across $SAMPLE_PAGES pages" >&2
  exit 2
fi

# ---------------------------------------------------------------------------
say "3. statistics and verdict"
# ---------------------------------------------------------------------------
read -r GAPS LO P50 P95 MAX PER6_PER100 LOWER CEILING VERDICT <<EOF
$(python3 - "$TMP" "$ASSUMED" "$MARGIN" "$MEAN" <<'PY'
import sys

path, assumed, margin, mean = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), float(sys.argv[4])
gaps = sorted(int(line) for line in open(path))
def pct(p):
    return gaps[int(p * (len(gaps) - 1))]
# Coverage is one-sided: the conversion is fully covering — a funded window
# spans at least the seconds it was funded for — for any real mean at or
# above assumed×100/(100+margin). Below that edge, windows shrink and the
# verdict is unsafe. Above assumed×(100+margin)/100 the conversion still
# over-funds (never unsafe), but the pinned constants no longer match the
# network: that is the re-pin edge, and flagging it is how a sustained
# change in close time becomes detectable.
lower = assumed * 100 / (100 + margin)
repin = assumed * (100 + margin) / 100
if mean < lower:
    verdict = "EXPOSED"
elif mean > repin:
    verdict = "STALE"
else:
    verdict = "COVERED"
per6 = sum(1 for g in gaps if g > assumed) * 100 // len(gaps)
print(len(gaps), gaps[0], pct(0.50), pct(0.95), gaps[-1], per6,
      f"{lower:.2f}", f"{repin:.2f}", verdict)
PY
)
EOF

echo "   observed mean:      ${MEAN} s/ledger  (exact, over all $WINDOW ledgers)"
echo "   sampled gaps:       $GAPS across $SAMPLE_PAGES pages"
echo "   per-ledger range:   ${LO} .. ${MAX} s   (p50 ${P50} s, p95 ${P95} s)"
echo "   gaps over nominal:  ${PER6_PER100}% of sampled gaps"
echo "   assumed:            ${ASSUMED} s  (storage::SECONDS_PER_LEDGER)"
echo "   safety margin:      ${MARGIN}%  -> coverage floor ${LOWER} s, re-pin above ${CEILING} s"
echo "   verdict:            $VERDICT"

if [[ "$VERDICT" == "COVERED" ]]; then
  echo
  echo "   The margin absorbs the observed close time. TTL targets funded by"
  echo "   seconds_to_ledgers last at least as long in wall-clock terms as"
  echo "   their schedule intends."
elif [[ "$VERDICT" == "STALE" ]]; then
  echo
  echo "   Safe but stale: the network now closes slower than the conversion"
  echo "   assumes, so every window over-funds — wasteful in rent, never"
  echo "   unsafe. Re-measure over the widest window available, then update"
  echo "   SECONDS_PER_LEDGER in contracts/stream/src/storage.rs,"
  echo "   docs/ledger-close-time.md, and the pin in test/ttl.rs together."
else
  echo
  echo "   UNSAFE: the observed mean sits below the coverage floor, so every"
  echo "   TTL target is shorter in wall-clock terms than its schedule and an"
  echo "   entry can become eligible to archive before its schedule ends."
  echo "   Re-measure over the widest window available, then update"
  echo "   SECONDS_PER_LEDGER in contracts/stream/src/storage.rs,"
  echo "   docs/ledger-close-time.md, and the pin in test/ttl.rs together."
fi

if $VERIFY && [[ "$VERDICT" != "COVERED" ]]; then
  echo
  echo "measure-ledger-close: --verify failed: margin does not cover the observation" >&2
  exit 1
fi
