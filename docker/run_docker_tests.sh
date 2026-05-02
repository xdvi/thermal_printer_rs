#!/usr/bin/env bash
# ==============================================================
# run_docker_tests.sh — Spin up the virtual printer and run
# integration tests against TCP port 9100.
#
# Usage:
#   chmod +x run_docker_tests.sh
#   ./docker/run_docker_tests.sh
# ==============================================================

set -euo pipefail

COMPOSE_FILE="$(dirname "$0")/docker-compose.yml"
PRINTER_HOST="127.0.0.1"
PRINTER_PORT="9100"

echo "--- Starting virtual ESC/POS printer..."
docker compose -f "$COMPOSE_FILE" up -d --wait

echo "--- Waiting for printer to be ready on $PRINTER_HOST:$PRINTER_PORT..."
for i in $(seq 1 10); do
  if nc -z "$PRINTER_HOST" "$PRINTER_PORT" 2>/dev/null; then
    echo "--- Printer ready."
    break
  fi
  sleep 1
done

echo "--- Running Rust integration tests (TCP against virtual printer)..."
cd "$(dirname "$0")/.."
RUST_LOG=info cargo test \
  --features tcp,codes_2d,barcodes \
  --test integration_test \
  -- --nocapture 2>&1

echo "--- Test run complete."
echo "--- View rendered tickets at: http://localhost:8181/output/"
echo "--- Stopping containers..."
docker compose -f "$COMPOSE_FILE" down
