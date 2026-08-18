#!/usr/bin/env bash
set -euo pipefail

SPARKLE_VERSION="2.9.6"
SPARKLE_SHA256="52bf9e88cdd972fc0c81501377a880e90d47031bd8ca5462488f843e2609e192"
SPARKLE_URL="https://github.com/sparkle-project/Sparkle/releases/download/$SPARKLE_VERSION/Sparkle-$SPARKLE_VERSION.tar.xz"

if [[ $# -ne 1 || -z "$1" || "$1" == "/" ]]; then
    echo "usage: $0 OUTPUT_DIRECTORY" >&2
    exit 2
fi

DESTINATION="$1"
PARENT="$(dirname "$DESTINATION")"
mkdir -p "$PARENT"
TEMP_DIR="$(mktemp -d "$PARENT/.sparkle-$SPARKLE_VERSION.XXXXXX")"
cleanup() {
    rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

ARCHIVE="$PARENT/Sparkle-$SPARKLE_VERSION.tar.xz"
EXPANDED="$TEMP_DIR/expanded"
mkdir -p "$EXPANDED"
if [[ ! -f "$ARCHIVE" ]]; then
    PARTIAL="$TEMP_DIR/Sparkle-$SPARKLE_VERSION.tar.xz.partial"
    curl --fail --location --silent --show-error "$SPARKLE_URL" -o "$PARTIAL"
    PARTIAL_SHA256="$(shasum -a 256 "$PARTIAL" | awk '{print $1}')"
    if [[ "$PARTIAL_SHA256" != "$SPARKLE_SHA256" ]]; then
        echo "Sparkle archive checksum mismatch" >&2
        exit 1
    fi
    mv "$PARTIAL" "$ARCHIVE"
fi
ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$SPARKLE_SHA256" ]]; then
    echo "Sparkle archive checksum mismatch" >&2
    exit 1
fi

tar -xJf "$ARCHIVE" -C "$EXPANDED"
test -f "$EXPANDED/Sparkle.framework/Versions/B/Sparkle"
test -f "$EXPANDED/Sparkle.framework/Headers/Sparkle.h"
test -x "$EXPANDED/bin/sign_update"
test -x "$EXPANDED/bin/generate_keys"
test -x "$EXPANDED/bin/generate_appcast"
test -f "$EXPANDED/LICENSE"

if [[ -e "$DESTINATION" ]]; then
    tree_digest() {
        (
            cd "$1"
            find . \( -type f -o -type l \) -print |
                LC_ALL=C sort |
                while IFS= read -r entry; do
                    if [[ -L "$entry" ]]; then
                        printf 'L\t%s\t%s\n' "$entry" "$(readlink "$entry")"
                    else
                        printf 'F\t%s\t%s\t%s\n' \
                            "$entry" \
                            "$(stat -f '%Lp' "$entry")" \
                            "$(shasum -a 256 "$entry" | awk '{print $1}')"
                    fi
                done
        ) | shasum -a 256 | awk '{print $1}'
    }
    if [[ ! -d "$DESTINATION" || -L "$DESTINATION" ]] ||
       [[ "$(tree_digest "$EXPANDED")" != "$(tree_digest "$DESTINATION")" ]]; then
        echo "refusing modified or incomplete Sparkle cache: $DESTINATION" >&2
        exit 1
    fi
    printf '%s\n' "$DESTINATION"
    exit 0
fi
mv "$EXPANDED" "$DESTINATION"
printf '%s\n' "$DESTINATION"
