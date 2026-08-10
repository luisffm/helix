#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

LABEL="${1:-run}"
DURATION="${2:-60}"
INTERVAL="${INTERVAL:-1}"
WARMUP="${WARMUP:-10}"
OUT_DIR="${OUT_DIR:-perf-results}"
mkdir -p "$OUT_DIR"

PID="${HELIX_PID:-$(pgrep -n -f 'target/(release|debug)/helix$|Helix.app/Contents/MacOS/helix' || true)}"
if [ -z "$PID" ]; then
  echo "perfstat: no running helix process found (set HELIX_PID=<pid>)" >&2
  exit 1
fi

GIT_REV="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
STAMP="$(date +%Y%m%d-%H%M%S)"
SUMMARY="$OUT_DIR/summary.tsv"

# Samples land outside the repo: writing them here once per second would be seen
# by the FS watcher of the very Helix that is being measured, and every batch
# costs it a git snapshot. Only the one summary line is written, after sampling.
SAMPLE_DIR="${TMPDIR:-/tmp}/helix-perfstat"
mkdir -p "$SAMPLE_DIR"
SAMPLES="$SAMPLE_DIR/$STAMP-$LABEL.samples"

BIN_BYTES=""
if [ -f target/release/helix ]; then
  BIN_BYTES="$(stat -f%z target/release/helix 2>/dev/null || stat -c%s target/release/helix)"
fi

# `ps -o pcpu` reports the average over the whole process lifetime, which for a
# freshly launched app is mostly startup. Sample cumulative CPU time instead and
# report the deltas, so a percentage means "of this sampling window".
cpu_seconds() {
  ps -o time= -p "$1" 2>/dev/null | awk -F: '
    NF == 0 { exit 1 }
    { s = $NF; if (NF >= 2) s += $(NF - 1) * 60; if (NF >= 3) s += $(NF - 2) * 3600; print s }
  '
}

rss_kb() {
  ps -o rss= -p "$1" 2>/dev/null | tr -d ' '
}

echo "perfstat: pid=$PID label=$LABEL warmup=${WARMUP}s duration=${DURATION}s interval=${INTERVAL}s rev=$GIT_REV"

sleep "$WARMUP"

PREV_CPU="$(cpu_seconds "$PID" || true)"
if [ -z "$PREV_CPU" ]; then
  echo "perfstat: process $PID exited during warmup" >&2
  exit 1
fi

PREV_AT="$(date +%s.%N)"
END=$((SECONDS + DURATION))

while [ $SECONDS -lt $END ]; do
  sleep "$INTERVAL"

  CPU="$(cpu_seconds "$PID" || true)"
  RSS="$(rss_kb "$PID" || true)"

  if [ -z "$CPU" ] || [ -z "$RSS" ]; then
    echo "perfstat: process $PID exited" >&2
    break
  fi

  NOW="$(date +%s.%N)"

  awk -v cpu="$CPU" -v prev="$PREV_CPU" -v now="$NOW" -v at="$PREV_AT" -v rss="$RSS" \
    'BEGIN { d = now - at; if (d > 0) printf "%s %.2f\n", rss, (cpu - prev) / d * 100 }' \
    >>"$SAMPLES"

  PREV_CPU="$CPU"
  PREV_AT="$NOW"
done

if [ ! -s "$SAMPLES" ]; then
  echo "perfstat: no samples collected" >&2
  exit 1
fi

read -r N CPU_AVG CPU_MAX RSS_AVG_MB RSS_MAX_MB <<<"$(awk '
  { n++; cpu += $2; rss += $1
    if ($2 > cpu_max) cpu_max = $2
    if ($1 > rss_max) rss_max = $1 }
  END { printf "%d %.1f %.1f %.1f %.1f", n, cpu / n, cpu_max, rss / n / 1024, rss_max / 1024 }
' "$SAMPLES")"

if [ ! -f "$SUMMARY" ]; then
  printf "stamp\trev\tlabel\tsamples\tcpu_avg_pct\tcpu_max_pct\trss_avg_mb\trss_max_mb\tbin_bytes\n" >"$SUMMARY"
fi
printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
  "$STAMP" "$GIT_REV" "$LABEL" "$N" "$CPU_AVG" "$CPU_MAX" "$RSS_AVG_MB" "$RSS_MAX_MB" "${BIN_BYTES:-na}" \
  >>"$SUMMARY"

echo "perfstat: $LABEL @ $GIT_REV -> cpu avg ${CPU_AVG}% max ${CPU_MAX}% | rss avg ${RSS_AVG_MB}MB max ${RSS_MAX_MB}MB | samples $N"
echo "perfstat: appended to $SUMMARY"
column -t -s "$(printf '\t')" "$SUMMARY" | tail -8
