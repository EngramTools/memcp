#!/usr/bin/env bash
# memcp status line for Claude Code
# Called by Claude Code via statusLine config. Reads session JSON from stdin (ignored),
# outputs a single-line daemon health indicator to stdout.
#
# Format is read from `memcp status` JSON (status_line.format in memcp.toml).
# Override with MEMCP_STATUSLINE env var: "ingest", "pending", or "state".

set -euo pipefail

# Claude Code sends session data on stdin — consume it
cat > /dev/null

# Pre-flight: jq is required for JSON parsing
if ! command -v jq &>/dev/null; then
    echo "memcp (no jq)"
    exit 0
fi

# Get memcp status JSON — fail silently if binary not found or DB unreachable
STATUS=$(memcp status --skip-migrate 2>/dev/null) || STATUS=""

if [ -z "$STATUS" ]; then
    echo "memcp ?"
    exit 0
fi

ALIVE=$(echo "$STATUS" | jq -r '.daemon.alive // false')

if [ "$ALIVE" != "true" ]; then
    # Dim text — daemon not running
    echo -e "\033[2mmemcp\033[0m"
    exit 0
fi

# Determine format: env var overrides config
CONFIG_FORMAT=$(echo "$STATUS" | jq -r '.status_line.format // "ingest"')
FORMAT="${MEMCP_STATUSLINE:-$CONFIG_FORMAT}"

case "$FORMAT" in
    pending)
        PENDING=$(echo "$STATUS" | jq -r '(.pending.embeddings + .pending.extractions) // 0')
        if [ "$PENDING" -gt 50 ]; then
            echo "⚠ memcp ${PENDING}⏳"
        elif [ "$PENDING" -gt 0 ]; then
            echo "✅ memcp ${PENDING}⏳"
        else
            echo "✅ memcp"
        fi
        ;;
    state)
        echo "✅ memcp"
        ;;
    ingest|*)
        LAST_INGEST=$(echo "$STATUS" | jq -r '.sidecar.last_ingest_at // empty')
        if [ -z "$LAST_INGEST" ]; then
            echo "✅ memcp"
        else
            # macOS-compatible relative time calculation
            if [[ "$OSTYPE" == "darwin"* ]]; then
                # BSD date: strip fractional seconds and timezone suffix for -jf parsing
                CLEAN_TS=$(echo "$LAST_INGEST" | sed 's/\.[0-9]*//; s/+00:00$//')
                INGEST_EPOCH=$(date -jf "%Y-%m-%dT%H:%M:%S" "$CLEAN_TS" +%s 2>/dev/null || echo "0")
            else
                INGEST_EPOCH=$(date -d "$LAST_INGEST" +%s 2>/dev/null || echo "0")
            fi
            NOW_EPOCH=$(date +%s)
            DIFF=$((NOW_EPOCH - INGEST_EPOCH))

            if [ "$INGEST_EPOCH" -eq 0 ]; then
                echo "✅ memcp"
            elif [ "$DIFF" -lt 60 ]; then
                echo "✅ memcp ${DIFF}s"
            elif [ "$DIFF" -lt 3600 ]; then
                echo "✅ memcp $((DIFF / 60))m"
            elif [ "$DIFF" -lt 86400 ]; then
                echo "✅ memcp $((DIFF / 3600))h"
            else
                echo "✅ memcp $((DIFF / 86400))d"
            fi
        fi
        ;;
esac
