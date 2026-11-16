#!/usr/bin/env bash
# One-time generator for crates/s3a-engine/tests/fixtures/test_tsa/: a throwaway test root CA,
# a test TSA certificate (RSA-2048, EKU id-kp-timeStamping) and two real RFC 3161 tokens over a
# fixed SHA-256 message imprint:
#   token_pkcs1_sha256.der  - signed by `openssl ts -reply` (RSASSA-PKCS1-v1_5, SHA-256)
#   token_pss_sha256.der    - same TSTInfo re-signed with `openssl cms` using RSASSA-PSS
#                             (SHA-256, MGF1-SHA-256, salt length 32)
# Private keys are generated in a temp dir and deleted; only public material is kept.
# Refuses to run if the output directory already exists (fixtures are frozen once committed).
# Usage (repo root): scripts/gen_test_tsa_fixtures.sh
set -euo pipefail
OUT="crates/s3a-engine/tests/fixtures/test_tsa"
if [ -e "$OUT" ]; then
  echo "refusing to overwrite existing $OUT (frozen fixtures)" >&2
  exit 1
fi
command -v openssl >/dev/null || { echo "openssl is required" >&2; exit 1; }
# Message imprint the tests expect: 32 bytes of 0x5a.
IMPRINT=$(printf '5a%.0s' $(seq 1 32))
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT

openssl req -x509 -newkey rsa:2048 -nodes -keyout "$T/ca.key" -out "$T/ca.crt" -days 3650 \
  -subj "/CN=S3A Test-Only Root CA (not trusted anywhere)" -sha256 \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -keyout "$T/tsa.key" -out "$T/tsa.csr" \
  -subj "/CN=S3A Test-Only TSA (not trusted anywhere)" 2>/dev/null
printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\n' > "$T/tsa.ext"
openssl x509 -req -in "$T/tsa.csr" -CA "$T/ca.crt" -CAkey "$T/ca.key" -CAcreateserial -days 3650 \
  -sha256 -extfile "$T/tsa.ext" -out "$T/tsa.crt" 2>/dev/null

cat > "$T/tsa.cnf" <<CNF
[ tsa ]
default_tsa = tsa_config
[ tsa_config ]
dir = $T
serial = $T/tsa_serial
signer_cert = $T/tsa.crt
certs = $T/ca.crt
signer_key = $T/tsa.key
signer_digest = sha256
default_policy = 1.3.6.1.4.1.99999.1
digests = sha256
accuracy = secs:1
ordering = no
tsa_name = no
ess_cert_id_chain = no
ess_cert_id_alg = sha256
CNF
echo 01 > "$T/tsa_serial"
openssl ts -query -digest "$IMPRINT" -sha256 -no_nonce -cert -out "$T/req.tsq"
openssl ts -reply -config "$T/tsa.cnf" -queryfile "$T/req.tsq" -token_out -out "$T/token_pkcs1.der" 2>/dev/null

# Extract the TSTInfo (eContent) and re-sign it as a PSS token.
openssl cms -verify -noverify -inform DER -in "$T/token_pkcs1.der" -out "$T/tstinfo.der" 2>/dev/null
openssl cms -sign -binary -nodetach -nosmimecap -outform DER -in "$T/tstinfo.der" \
  -econtent_type 1.2.840.113549.1.9.16.1.4 -signer "$T/tsa.crt" -inkey "$T/tsa.key" \
  -certfile "$T/ca.crt" -md sha256 -keyopt rsa_padding_mode:pss -keyopt rsa_pss_saltlen:32 \
  -keyopt rsa_mgf1_md:sha256 -out "$T/token_pss.der"

# Sanity: OpenSSL itself verifies both tokens against the test root.
openssl ts -verify -token_in -in "$T/token_pkcs1.der" -digest "$IMPRINT" -CAfile "$T/ca.crt" >/dev/null
openssl cms -verify -inform DER -in "$T/token_pss.der" -CAfile "$T/ca.crt" -purpose any -out /dev/null 2>/dev/null

mkdir -p "$OUT"
cp "$T/ca.crt" "$OUT/test_root_ca.pem"
cp "$T/tsa.crt" "$OUT/test_tsa_cert.pem"
cp "$T/token_pkcs1.der" "$OUT/token_pkcs1_sha256.der"
cp "$T/token_pss.der" "$OUT/token_pss_sha256.der"
echo "$IMPRINT" > "$OUT/message_imprint_sha256.hex"
cat > "$OUT/README.md" <<README
Test-only RFC 3161 fixtures, generated once by scripts/gen_test_tsa_fixtures.sh
($(openssl version | cut -d' ' -f1-2), $(date -u +%Y-%m-%dT%H:%M:%SZ)). The CA and TSA private
keys were deleted after signing. Nothing here is trusted outside these tests.
README
(cd "$OUT" && sha256sum *.der *.pem *.hex)
