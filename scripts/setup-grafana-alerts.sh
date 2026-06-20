#!/bin/bash
# Create Pillar fleet alert rules in Grafana via the provisioning API.
#
# Rules evaluate against the Pillar Prometheus metrics. They use Grafana's default
# notification policy/contact point (no working integration = notifications go nowhere)
# until you wire a Slack / PagerDuty / Telegram contact point — see docs/OPERATIONS.md §6.
#
# Run on the controller host (where Grafana listens on :3000). Idempotent-ish: re-running
# creates duplicate rules, so delete existing ones first if re-provisioning.
set -e

GRAFANA="${GRAFANA:-http://localhost:3000}"
# Resolve the Pillar dashboards folder UID (created by install-controller.sh).
FOLDER=$(curl -s -m8 "$GRAFANA/api/folders" | python3 -c "import sys,json;print(next((f['uid'] for f in json.load(sys.stdin) if f.get('title')=='Pillar'), ''))")
[ -z "$FOLDER" ] && { echo "ERROR: 'Pillar' folder not found in Grafana"; exit 1; }

post_rule() {
  local title="$1" expr="$2" evaltype="$3" thresh="$4" sev="$5" summary="$6" forr="$7"
  curl -s -o /dev/null -w "%{http_code}  $title\n" \
    -X POST "$GRAFANA/api/v1/provisioning/alert-rules" \
    -H "Content-Type: application/json" -H "X-Disable-Provenance: true" \
    -d "{
      \"title\": \"$title\", \"ruleGroup\": \"pillar-fleet\", \"folderUID\": \"$FOLDER\",
      \"orgID\": 1, \"condition\": \"C\", \"for\": \"$forr\",
      \"noDataState\": \"OK\", \"execErrState\": \"Error\", \"isPaused\": false,
      \"labels\": {\"severity\": \"$sev\", \"team\": \"pillar\"},
      \"annotations\": {\"summary\": \"$summary\"},
      \"data\": [
        {\"refId\":\"A\",\"relativeTimeRange\":{\"from\":600,\"to\":0},\"datasourceUid\":\"pillar-prometheus\",\"model\":{\"expr\":\"$expr\",\"instant\":true,\"intervalMs\":1000,\"maxDataPoints\":43200,\"refId\":\"A\"}},
        {\"refId\":\"C\",\"relativeTimeRange\":{\"from\":600,\"to\":0},\"datasourceUid\":\"__expr__\",\"model\":{\"type\":\"threshold\",\"expression\":\"A\",\"conditions\":[{\"evaluator\":{\"type\":\"$evaltype\",\"params\":[$thresh]}}],\"refId\":\"C\",\"intervalMs\":1000,\"maxDataPoints\":43200}}
      ]
    }"
}

post_rule "Validator unhealthy / offline" "pillar_node_healthy" "lt" "1" "critical" "Validator {{ \$labels.node_id }} is unhealthy or offline" "2m"
post_rule "Validator lagging behind" "pillar_node_slots_behind" "gt" "5000" "warning" "Validator {{ \$labels.node_id }} is more than 5000 slots behind" "5m"
post_rule "Validator restart loop" "increase(pillar_node_restarts_total[15m])" "gt" "3" "warning" "Validator {{ \$labels.node_id }} is restarting repeatedly" "5m"
post_rule "Validator disk almost full" "pillar_node_disk_used_bytes / pillar_node_disk_total_bytes" "gt" "0.9" "warning" "Validator {{ \$labels.node_id }} disk is over 90% full" "10m"
echo "DONE"
