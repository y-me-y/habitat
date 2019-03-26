#!/bin/bash

set -eou pipefail

sudo hab pkg install core/rust
sudo hab pkg install core/cacerts
export SSL_CERT_FILE=/usr/local/etc/openssl/cert.pem
