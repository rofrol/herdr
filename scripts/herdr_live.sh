#!/usr/bin/env bash
# Try this checkout's release build in the running Herdr session, then install
# it or go back. The installed binary is the last known-good build.
set -euo pipefail

usage() {
  cat <<'USAGE'
usage: herdr_live.sh test|keep|back

  test  hand the session off to this checkout's target/release/herdr and
        record its hash; attach with that binary afterwards
  keep  install the tested build over the installed binary (refuses if the
        build changed since `test`) and hand the session off to it
  back  hand the session back to the installed binary

The installed binary is $HERDR_INSTALLED (default ~/.cargo/bin/herdr).
USAGE
}

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
candidate="$repo/target/release/herdr"
installed="${HERDR_INSTALLED:-$HOME/.cargo/bin/herdr}"
tested="$HOME/.cache/herdr/tested"

handoff() {
  "$installed" server live-handoff --import-exe "$1"
  echo "attach with: $2"
}

fingerprint() {
  printf '%s %s\n' "$(shasum -a 256 "$candidate" | cut -d' ' -f1)" "$candidate"
}

case "${1:-}" in
  test)
    [[ -x "$candidate" ]] || { echo "no build at $candidate; run cargo build --release --locked" >&2; exit 1; }
    mkdir -p "$(dirname "$tested")"
    fingerprint >"$tested"
    handoff "$candidate" "$candidate"
    ;;
  keep)
    if [[ "$(fingerprint)" != "$(cat "$tested" 2>/dev/null)" ]]; then
      echo "$candidate is not the build last tried with \`test\`" >&2
      exit 1
    fi
    # A new file renamed over the old one: a running server keeps its binary,
    # and nothing ever sees a half-copied file.
    tmp="$(mktemp "$installed.XXXXXX")"
    trap 'rm -f "$tmp"' EXIT
    cp "$candidate" "$tmp"
    chmod 755 "$tmp"
    mv -f "$tmp" "$installed"
    handoff "$installed" herdr
    ;;
  back)
    handoff "$installed" herdr
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
