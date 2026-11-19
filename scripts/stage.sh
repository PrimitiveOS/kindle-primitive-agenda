#!/bin/sh
# stage.sh [armhf|armel|RUST_TARGET] [STAGE_DIR]
set -eu
cd "$(dirname "$0")/.."
case "${1:-armhf}" in
    armhf|armv7-unknown-linux-musleabihf) target=armv7-unknown-linux-musleabihf ;;
    armel|armv7-unknown-linux-musleabi) target=armv7-unknown-linux-musleabi ;;
    *) echo "Unsupported package architecture: $1" >&2; exit 1 ;;
esac
stage=${2:-build/stage}
target_dir=${CARGO_TARGET_DIR:-target}
bin=$target_dir/$target/release/agenda-kindle
[ -x "$bin" ] || { echo "Build $bin before staging." >&2; exit 1; }
install -d "$stage/agenda/bin" "$stage/agenda/licenses" "$stage/documents"
install -m 755 "$bin" "$stage/agenda/bin/agenda-kindle"
install -m 755 deploy/Agenda.sh "$stage/documents/Agenda.sh"
install -m 644 licenses/chrono-LICENSE.txt "$stage/agenda/licenses/chrono-LICENSE.txt"
