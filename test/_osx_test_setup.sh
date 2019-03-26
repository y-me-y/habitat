#!/bin/bash

set -eou pipefail

export SSL_CERT_FILE=/usr/local/etc/openssl/cert.pem

sudo hab pkg install core/rust
