#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TLS_DIR="${ROOT_DIR}/.tls"

CA_KEY="${TLS_DIR}/bvb-dev-root-ca.key"
CA_CERT="${TLS_DIR}/bvb-dev-root-ca.crt"
CA_SRL="${TLS_DIR}/bvb-dev-root-ca.srl"
SERVER_KEY="${TLS_DIR}/bvb-lan.key"
SERVER_CSR="${TLS_DIR}/bvb-lan.csr"
SERVER_CERT="${TLS_DIR}/bvb-lan.crt"
OPENSSL_CFG="${TLS_DIR}/openssl-lan.cnf"
LAN_IP_FILE="${TLS_DIR}/lan-ip.txt"

detect_lan_ip() {
  local default_iface
  local candidate_ip

  default_iface="$(route -n get default 2>/dev/null | awk '/interface:/{print $2; exit}')"
  if [[ -n "${default_iface}" ]]; then
    candidate_ip="$(ipconfig getifaddr "${default_iface}" 2>/dev/null || true)"
    if [[ -n "${candidate_ip}" ]]; then
      echo "${candidate_ip}"
      return 0
    fi
  fi

  candidate_ip="$(ifconfig | awk '/inet / && $2 !~ /^127\./ {print $2; exit}')"
  echo "${candidate_ip:-}"
}

LAN_IP="${BVB_LAN_IP:-$(detect_lan_ip)}"
if [[ -z "${LAN_IP}" ]]; then
  echo "Could not detect LAN IPv4 address. Set BVB_LAN_IP and retry." >&2
  exit 1
fi

mkdir -p "${TLS_DIR}"

if [[ ! -f "${CA_KEY}" || ! -f "${CA_CERT}" ]]; then
  openssl genrsa -out "${CA_KEY}" 4096 >/dev/null 2>&1
  openssl req -x509 -new -nodes -key "${CA_KEY}" \
    -sha256 -days 3650 \
    -subj "/CN=Beyond vs Below Dev Root CA" \
    -out "${CA_CERT}" >/dev/null 2>&1
fi

cat > "${OPENSSL_CFG}" <<EOF
[req]
distinguished_name = req_distinguished_name
prompt = no
req_extensions = req_ext

[req_distinguished_name]
CN = bvb-lan

[req_ext]
subjectAltName = @alt_names
extendedKeyUsage = serverAuth

[alt_names]
DNS.1 = localhost
IP.1 = 127.0.0.1
IP.2 = ${LAN_IP}
EOF

openssl genrsa -out "${SERVER_KEY}" 2048 >/dev/null 2>&1
openssl req -new -key "${SERVER_KEY}" -out "${SERVER_CSR}" \
  -config "${OPENSSL_CFG}" >/dev/null 2>&1
openssl x509 -req -in "${SERVER_CSR}" \
  -CA "${CA_CERT}" -CAkey "${CA_KEY}" -CAcreateserial \
  -CAserial "${CA_SRL}" \
  -out "${SERVER_CERT}" \
  -days 825 -sha256 \
  -extensions req_ext -extfile "${OPENSSL_CFG}" >/dev/null 2>&1

rm -f "${SERVER_CSR}" "${OPENSSL_CFG}"
echo "${LAN_IP}" > "${LAN_IP_FILE}"

echo "LAN_IP=${LAN_IP}"
echo "CA_CERT=${CA_CERT}"
echo "SERVER_CERT=${SERVER_CERT}"
echo "SERVER_KEY=${SERVER_KEY}"
