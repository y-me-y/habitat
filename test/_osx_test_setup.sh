#!/bin/bash

set -eou pipefail

sudo hab pkg install core/rust
export SSL_CERT_FILE=/usr/local/etc/openssl/cert.pem
