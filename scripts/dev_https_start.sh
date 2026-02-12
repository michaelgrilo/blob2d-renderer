#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SESSION_NAME="${BVB_DEV_SESSION:-bvb-https}"
PORT="${BVB_DEV_PORT:-4173}"
ADDRESS="${BVB_DEV_ADDRESS:-0.0.0.0}"

"${ROOT_DIR}/scripts/dev_https_cert.sh" >/dev/null

LAN_IP_FILE="${ROOT_DIR}/.tls/lan-ip.txt"
if [[ ! -f "${LAN_IP_FILE}" ]]; then
  echo "Missing LAN IP metadata: ${LAN_IP_FILE}" >&2
  exit 1
fi
LAN_IP="$(cat "${LAN_IP_FILE}")"

if tmux has-session -t "${SESSION_NAME}" 2>/dev/null; then
  echo "tmux session '${SESSION_NAME}' is already running."
else
  if lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "Port ${PORT} is already in use. Stop the current listener first." >&2
    lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN >&2 || true
    exit 1
  fi

  tmux new-session -d -s "${SESSION_NAME}" \
    "cd '${ROOT_DIR}' && env NO_COLOR=false TRUNK_NO_COLOR=true trunk serve --address ${ADDRESS} --port ${PORT} --no-autoreload --tls-key-path .tls/bvb-lan.key --tls-cert-path .tls/bvb-lan.crt --ws-protocol wss"

  sleep 1
  if ! tmux has-session -t "${SESSION_NAME}" 2>/dev/null; then
    echo "Failed to start tmux session '${SESSION_NAME}'." >&2
    exit 1
  fi
fi

echo "session: ${SESSION_NAME}"
echo "local:   https://127.0.0.1:${PORT}"
echo "lan:     https://${LAN_IP}:${PORT}"
echo "ca:      ${ROOT_DIR}/.tls/bvb-dev-root-ca.crt"
echo "logs:    tmux capture-pane -pt ${SESSION_NAME}:0 -S -200"
echo "stop:    ${ROOT_DIR}/scripts/dev_https_stop.sh"
