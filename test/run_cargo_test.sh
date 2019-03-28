#!/bin/bash

set -eoxu pipefail

while [[ $# -gt 1 ]]; do
  case $1 in
    -f | --features )       shift
                            features=$1
                            ;;
    -t | --test-options )   shift
                            test_options=$1
                            ;;
    * )                     echo "FAIL SCHOONER"
                            exit 1
  esac
  shift
done

# set the features string if needed
[ -z "${features:-}" ] && features_string="" || features_string="--features ${features}"

component=${1?component argument required}
cargo_test_command="cargo test ${features_string} -- --nocapture ${test_options:-}"

# TODO: fix this upstream so it's already on the path and set up
# export RUSTUP_HOME=/opt/rust
# export CARGO_HOME=/home/buildkite-agent/.cargo
# export PATH=/opt/rust/bin:$PATH
# TODO: fix this upstream, it looks like it's not saving correctly.
# sudo chown -R buildkite-agent /home/buildkite-agent

# . ./test/_osx_test_setup.sh

export SSL_CERT_FILE=/usr/local/etc/openssl/cert.pem
curl https://s3-us-west-2.amazonaws.com/shain-bk-test/mac-bootstrapper-1.0.0-latest.pkg -o mac-bootstrapper-1.0.0-latest.pkg
sudo installer -pkg mac-bootstrapper-1.0.0-latest.pkg -target /

curl https://sh.rustup.rs -sSf | sh -s -- -y
. $HOME/.cargo/env
rustup install stable
rustup default stable

# sudo hab pkg install core/rust

# TODO: these should be in a shared script?
# sudo hab pkg install core/bzip2
# sudo hab pkg install core/libarchive
# sudo hab pkg install core/libsodium
# sudo hab pkg install core/openssl
# sudo hab pkg install core/xz
# sudo hab pkg install core/zeromq
# sudo hab pkg install core/protobuf --binlink

export SODIUM_STATIC=true # so the libarchive crate links to sodium statically
export LIBARCHIVE_STATIC=true # so the libarchive crate *builds* statically
export OPENSSL_DIR # so the openssl crate knows what to build against
OPENSSL_DIR=/opt/mac-bootstrapper/embedded/lib
export OPENSSL_STATIC=true # so the openssl crate builds statically
export LIBZMQ_PREFIX
LIBZMQ_PREFIX=/opt/mac-bootstrapper/embedded/lib
# now include openssl and zeromq so thney exists in the runtime library path when cargo test is run
export LD_LIBRARY_PATH
LD_LIBRARY_PATH=/opt/mac-bootstrapper/embedded/lib
# include these so that the cargo tests can bind to libarchive (which dynamically binds to xz, bzip, etc), openssl, and sodium at *runtime*
export LIBRARY_PATH
LIBRARY_PATH=/opt/mac-bootstrapper/embedded/lib
# setup pkgconfig so the libarchive crate can use pkg-config to fine bzip2 and xz at *build* time
export PKG_CONFIG_PATH
PKG_CONFIG_PATH=/opt/mac-bootstrapper/embedded/lib/pkgconfig

# Set testing filesystem root
export TESTING_FS_ROOT
TESTING_FS_ROOT=$(mktemp -d /tmp/testing-fs-root-XXXXXX)
echo "--- Running cargo test on $component with command: '$cargo_test_command'"
cd "components/$component"
$cargo_test_command
