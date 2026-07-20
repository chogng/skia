#!/usr/bin/env bash

set -euo pipefail

destination="${1:-target/unicode-conformance}"
mkdir -p "$destination"

download() {
    local url="$1"
    local filename="$2"
    local expected_sha256="$3"
    local output="$destination/$filename"

    if [[ -f "$output" ]] && [[ "$(shasum -a 256 "$output" | awk '{print $1}')" == "$expected_sha256" ]]; then
        return
    fi

    local temporary
    temporary="$(mktemp "$destination/.${filename}.XXXXXX")"
    trap 'rm -f "$temporary"' RETURN
    curl --fail --location --silent --show-error --output "$temporary" "$url"
    local actual_sha256
    actual_sha256="$(shasum -a 256 "$temporary" | awk '{print $1}')"
    if [[ "$actual_sha256" != "$expected_sha256" ]]; then
        echo "checksum mismatch for $filename" >&2
        return 1
    fi
    mv "$temporary" "$output"
    trap - RETURN
}

download \
    "https://www.unicode.org/Public/17.0.0/ucd/auxiliary/GraphemeBreakTest.txt" \
    "GraphemeBreakTest-17.0.0.txt" \
    "e2d134d2c52919bace503ebb6a551c1855fe1a1faec18478c78fff254a1793ec"
download \
    "https://www.unicode.org/Public/15.0.0/ucd/auxiliary/LineBreakTest.txt" \
    "LineBreakTest-15.0.0.txt" \
    "371bde4052aa593b108684ae292d8ea2dbb93c19990e0cdf416fa7239557aac3"
download \
    "https://www.unicode.org/Public/16.0.0/ucd/BidiCharacterTest.txt" \
    "BidiCharacterTest-16.0.0.txt" \
    "d04a51a90052dcd71c4e91ee5b3a9d973ee35c12406b5a99875ac8163c8f2804"

echo "Unicode conformance data is ready in $destination"
