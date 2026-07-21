#!/usr/bin/env bash

set -euo pipefail

if [[ "${SKIA_UPDATE_GOLDENS:-}" != "1" ]]; then
    echo "refusing to rewrite goldens; set SKIA_UPDATE_GOLDENS=1" >&2
    exit 2
fi

cargo test -p skia-gpu --features software --test render_oracle regenerate_owned_goldens -- --ignored --exact
