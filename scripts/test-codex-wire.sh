#!/usr/bin/env bash
# Host-side unit tests for the firmware's Codex Micro protocol codec
# (fw/src/codex/wire.rs). The firmware crate only builds for thumbv6m, so the
# codec is compiled into a tiny host crate and tested against the request /
# reply payloads the reference projects captured from ChatGPT Desktop.
#
#   scripts/test-codex-wire.sh          # cargo test, host target
#
# fw/.cargo/config.toml pins the build target to thumbv6m for everything
# under fw/, so the host triple is passed explicitly.
set -euo pipefail
cd "$(dirname "$0")/.."
host="$(rustc -vV | sed -n 's/^host: //p')"
exec cargo test --manifest-path fw/host-tests/Cargo.toml --target "$host" "$@"
