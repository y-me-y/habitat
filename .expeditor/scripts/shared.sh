#!/bin/bash

set -euo pipefail

get_release_channel() {
    echo "habitat-release-${BUILDKITE_BUILD_ID}"
}
