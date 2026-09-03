#!/usr/bin/env bash
# Export archived logs as JSON and serve them over HTTP for Grafana's
# Infinity datasource plugin (https://grafana.com/grafana/plugins/yesoreyeram-infinity-datasource/)
# to poll. Infinity reads from a URL, not a local file or a command — this
# script is the small bridge that makes ObsidianLog's CLI output pollable.
#
# One-shot by design (this is an example, not a production daemon). For
# continuous refresh, wrap it in `watch -n 60 ./export-and-serve.sh`, a cron
# job, or a systemd timer.
#
# Usage:
#   ./export-and-serve.sh [config-path] [port]
#
# Then in Grafana: add an Infinity datasource pointed at
#   http://localhost:<port>/logs.json
# (see datasource.yaml for a provisioning-style example), and import
# dashboard.json to see it visualized.

set -euo pipefail

CONFIG_PATH="${1:-}"
PORT="${2:-8787}"
OUT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/export"

mkdir -p "$OUT_DIR"

echo "Exporting query results to $OUT_DIR/logs.json..."
if [[ -n "$CONFIG_PATH" ]]; then
  obsidianlog query --config "$CONFIG_PATH" --format json > "$OUT_DIR/logs.json"
else
  obsidianlog query --format json > "$OUT_DIR/logs.json"
fi

echo "Serving $OUT_DIR at http://localhost:$PORT/logs.json"
echo "Press Ctrl+C to stop."
cd "$OUT_DIR"
python3 -m http.server "$PORT"
