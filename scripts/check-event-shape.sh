#!/usr/bin/env bash
# check-event-shape.sh
# Verifies that every env.events().publish() call site across all contracts
# uses a centralized events::event_*() constructor (never inline Symbol::new).
# Also verifies that every constructor-exported topic is snapshot-tested and
# appears in EVENT_TOPICS.md.
#
# Version markers (`event_version_*`, e.g. "callora_v1") describe the event
# schema version rather than an action, are emitted next to topic 0 instead of
# replacing it, and are therefore excluded from the snapshot-test and
# documentation rules. The script still reports them so an invalid marker
# literal (one that is not a valid Soroban symbol) stays visible.
# Issue #1118: additionally verifies that every vault publish() call site
# includes the version constructor (events::event_version_v1) at topic[1].
#
# Exit 0 = all checks pass. Exit 1 = any check fails.
set -euo pipefail

SCHEMA="docs/EVENT_TOPICS.md"

# Soroban topic strings are lowercase identifiers (see
# tests/event_topic_catalog.rs::all_topic_strings_are_valid_identifiers).
TOPIC_RE='^[a-z_][a-z0-9_]*$'

if [[ ! -f "$SCHEMA" ]]; then
  echo "ERROR: $SCHEMA not found (run from repo root)" >&2
  exit 1
fi

FAIL=0

check_contract() {
  local contract_name="$1"
  local lib="contracts/${contract_name}/src/lib.rs"
  local events="contracts/${contract_name}/src/events.rs"

  if [[ ! -f "$lib" ]]; then
    echo "WARN: $lib not found, skipping $contract_name" >&2
    return
  fi

  echo "=== $contract_name Event Shape Check ==="

  # 1. Verify no inline Symbol::new in publish call sites
  local INLINE_COUNT
  INLINE_COUNT=$(grep -cP 'env\.events\(\)\.publish\(\(.*Symbol::new' "$lib" 2>/dev/null || true)
  if [[ "$INLINE_COUNT" -gt 0 ]]; then
    echo "FAIL: $contract_name has $INLINE_COUNT inline Symbol::new in publish() calls"
    grep -nP 'env\.events\(\)\.publish\(\(.*Symbol::new' "$lib" || true
    FAIL=1
  else
    echo "OK: no inline Symbol::new in publish() calls"
  fi

  if [[ -f "$events" ]]; then
    # Constructors that produce a real event topic (version markers excluded).
    local CTOR_COUNT
    CTOR_COUNT=$( { grep -oP 'pub fn \Kevent_(?!version)\w+' "$events" || true; } | wc -l | tr -d ' ')

    # Topic literals produced by the constructors above. The whole literal is
    # captured so a malformed symbol (e.g. one containing ".") cannot silently
    # truncate into a different string.
    local PRODUCED_TOPICS TESTED_TOPICS MARKERS
    # Version-marker literals are identified by their constructor name
    # (`event_version_*`), not by their spelling, so both the legacy invalid
    # "callora.v1" and the valid "callora_v1" are excluded from topic checks.
    MARKERS=$( { grep -A3 -P 'pub fn event_version\w*' "$events" || true; } | { grep -oP 'Symbol::new\(env,\s*"\K[^"]*' || true; } | sort -u || true)
    PRODUCED_TOPICS=$( { grep -oP 'Symbol::new\(env,\s*"\K[^"]*' "$events" || true; } | sort -u | { grep -vxF -f <(printf '%s\n' "$MARKERS" | sed '/^$/d'; echo '__no_marker__') || true; })
    TESTED_TOPICS=$( { grep -oP 'Symbol::new\(&env,\s*"\K[^"]*' "$events" || true; } | sort -u | { grep -vxF -f <(printf '%s\n' "$MARKERS" | sed '/^$/d'; echo '__no_marker__') || true; })

    local PRODUCED_COUNT TESTED_COUNT
    PRODUCED_COUNT=$(echo "$PRODUCED_TOPICS" | sed '/^$/d' | wc -l | tr -d ' ')
    TESTED_COUNT=$(echo "$TESTED_TOPICS" | sed '/^$/d' | wc -l | tr -d ' ')

    # 2a. Every constructor must publish its own distinct topic literal.
    if [[ "$CTOR_COUNT" -ne "$PRODUCED_COUNT" ]]; then
      echo "FAIL: $contract_name has $CTOR_COUNT constructors but only $PRODUCED_COUNT distinct topic literals"
      FAIL=1
    else
      echo "OK: $CTOR_COUNT constructors map to $PRODUCED_COUNT distinct topics"
    fi

    # 2b. Every produced topic must be covered by a snapshot test.
    local UNTESTED=()
    local topic
    while IFS= read -r topic; do
      [[ -z "$topic" ]] && continue
      if ! printf '%s\n' "$TESTED_TOPICS" | grep -qx "$topic"; then
        UNTESTED+=("$topic")
      fi
    done <<< "$PRODUCED_TOPICS"

    if [[ ${#UNTESTED[@]} -gt 0 ]]; then
      echo "FAIL: the following $contract_name topics have no snapshot test in events.rs:"
      for topic in "${UNTESTED[@]}"; do
        echo "  - $topic"
      done
      FAIL=1
    else
      echo "OK: all $PRODUCED_COUNT topics are snapshot-tested ($TESTED_COUNT unique tested)"
    fi

    # 2c. Report version markers, and flag marker literals that are not valid
    # Soroban symbols (they panic at runtime and can never be snapshot-tested).
    local marker
    while IFS= read -r marker; do
      [[ -z "$marker" ]] && continue
      if [[ ! "$marker" =~ $TOPIC_RE ]]; then
        echo "WARN: $contract_name version marker \"$marker\" is not a valid Soroban symbol"
      else
        echo "OK: $contract_name version marker \"$marker\""
      fi
    done <<< "$MARKERS"

    # 3. Verify every constructor-exported topic appears in EVENT_TOPICS.md
    mapfile -t SCHEMA_TOPICS < <(
      grep -oP '^\|\s*\d+\s*\|\s*`\K[^`]+' "$SCHEMA" | sort -u
    )

    local MISSING=()
    while IFS= read -r topic; do
      [[ -z "$topic" ]] && continue
      if ! printf '%s\n' "${SCHEMA_TOPICS[@]}" | grep -qx "$topic"; then
        MISSING+=("$topic")
      fi
    done <<< "$PRODUCED_TOPICS"

    if [[ ${#MISSING[@]} -gt 0 ]]; then
      echo "FAIL: the following $contract_name topics are in events.rs but not in EVENT_TOPICS.md:"
      for m in "${MISSING[@]}"; do
        echo "  - $m"
      done
      FAIL=1
    else
      echo "OK: all $PRODUCED_COUNT topics documented in EVENT_TOPICS.md"
    fi
  fi

  echo ""
}

# Issue #1118: verify every vault publish() call includes event_version_v1.
# Strategy: count publish() calls vs publish() calls that also reference
# event_version_v1 in the same multi-line block (up to 6 lines ahead).
check_vault_version_topics() {
  local lib="contracts/vault/src/lib.rs"
  echo "=== vault Version-Topic Check (Issue #1118) ==="

  # Count publish blocks that lack event_version_v1.
  # Each publish( opens a tuple; we collect lines until the closing );
  # and check whether event_version_v1 appears in those lines.
  local MISSING
  MISSING=$(python3 - "$lib" <<'PYEOF'
import re, sys

text = open(sys.argv[1]).read()
lines = text.splitlines()

violations = []
i = 0
while i < len(lines):
    line = lines[i]
    if 'env.events().publish(' in line:
        # Collect this block until we see ); at start of a line (end of call)
        block_lines = [line]
        j = i + 1
        while j < len(lines) and j < i + 20:
            block_lines.append(lines[j])
            if lines[j].strip().startswith(');'):
                break
            j += 1
        block = '\n'.join(block_lines)
        if 'event_version_v1' not in block:
            violations.append(f"  line {i+1}: {line.strip()[:80]}")
    i += 1

for v in violations:
    print(v)
print(len(violations))
PYEOF
  )

  local COUNT
  COUNT=$(echo "$MISSING" | tail -1)
  local DETAILS
  DETAILS=$(echo "$MISSING" | head -n -1)

  if [[ "$COUNT" -gt 0 ]]; then
    echo "FAIL: $COUNT vault publish() call(s) missing event_version_v1:"
    echo "$DETAILS"
    FAIL=1
  else
    echo "OK: all vault publish() calls include event_version_v1"
  fi
  echo ""
}

check_contract "vault"
check_contract "settlement"
check_contract "revenue_pool"
check_contract "distribute"
check_vault_version_topics

if [[ "$FAIL" -ne 0 ]]; then
  echo "FAILED: some contracts have undocumented or unversioned events. See above."
  exit 1
fi

echo "OK: all event constructors are centralized, tested, and documented."
exit 0
