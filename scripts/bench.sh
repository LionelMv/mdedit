#!/usr/bin/env bash
# GUI benchmark, same method as the WordCraft baseline in docs/architecture.md §5.0:
# launch the app with a file, sample /proc/<pid>/status VmRSS and /proc/<pid>/stat
# utime+stime every 0.25 s, then report peak RSS, settled RSS, CPU used, settle time
# and idle CPU over the last 5 s. The window must be left untouched while it runs.
#
# Usage: scripts/bench.sh BINARY FILE [SECONDS]      (default 20 s; minimum 6)
# Example:
#   python3 -I scripts/gen-big.py /tmp/mdbench
#   scripts/bench.sh target/release/mdedit /tmp/mdbench/big1m.md 20
# Extra arguments for the binary can be given in BENCH_ARGS (word-split).
set -euo pipefail

if [[ $# -lt 2 ]]; then
    sed -n '2,11p' "$0" >&2
    exit 2
fi

BIN="$1"
FILE="$2"
SECONDS_TO_RUN="${3:-20}"

[[ -x "${BIN}" ]] || { echo "error: not executable: ${BIN}" >&2; exit 1; }
[[ -f "${FILE}" ]] || { echo "error: no such file: ${FILE}" >&2; exit 1; }

# shellcheck disable=SC2206
EXTRA=(${BENCH_ARGS:-})

python3 -I - "${SECONDS_TO_RUN}" "${BIN}" "${EXTRA[@]}" "${FILE}" <<'PY'
import os
import signal
import subprocess
import sys
import time

INTERVAL = 0.25
IDLE_WINDOW = 5.0

duration = float(sys.argv[1])
cmd = sys.argv[2:]
if duration < IDLE_WINDOW + 1:
    sys.exit(f"duration must be at least {IDLE_WINDOW + 1:.0f} s")

ticks = os.sysconf("SC_CLK_TCK")


def rss_kb(pid):
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
    return 0


def cpu_s(pid):
    with open(f"/proc/{pid}/stat") as f:
        data = f.read()
    # comm (field 2) may contain spaces; fields after ')' start at field 3 (state).
    fields = data[data.rindex(")") + 2:].split()
    return (int(fields[11]) + int(fields[12])) / ticks  # utime, stime


proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
start = time.monotonic()
samples = []  # (t, rss_kb, cpu_s)
try:
    while True:
        t = time.monotonic() - start
        if t > duration:
            break
        if proc.poll() is not None:
            break
        try:
            samples.append((t, rss_kb(proc.pid), cpu_s(proc.pid)))
        except (FileNotFoundError, ProcessLookupError):
            break
        next_t = start + (len(samples)) * INTERVAL
        time.sleep(max(0.0, next_t - time.monotonic()))
finally:
    exited_early = proc.poll() is not None
    if not exited_early:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    stderr_tail = proc.stderr.read().decode(errors="replace")[-2000:] if proc.stderr else ""

if exited_early:
    print(f"warning: process exited early with code {proc.returncode}", file=sys.stderr)
    if stderr_tail:
        print(stderr_tail, file=sys.stderr)
if len(samples) < 2:
    sys.exit("not enough samples")

t_end, rss_end, cpu_end = samples[-1]
peak = max(s[1] for s in samples)

# Idle CPU: CPU-seconds per second over the last IDLE_WINDOW seconds.
base = next((s for s in samples if s[0] >= t_end - IDLE_WINDOW), samples[0])
window = t_end - base[0]
idle = (cpu_end - base[2]) / window if window > 0 else 0.0

# Settle time: first sample after which CPU grows by < 0.05 CPU-s/s for the rest of the run.
settle = None
for i, (t, _, c) in enumerate(samples):
    rest = samples[i:]
    if all((c2 - c) <= 0.05 * (t2 - t) + 0.02 for t2, _, c2 in rest):
        settle = t
        break

print(f"command:            {' '.join(cmd)}")
size = os.path.getsize(cmd[-1])
print(f"file size:          {size / 1e6:.2f} MB" if size >= 100_000 else f"file size:          {size / 1e3:.1f} KB")
print(f"samples:            {len(samples)} over {t_end:.1f} s (every {INTERVAL} s)")
print(f"peak RSS:           {peak / 1024:.1f} MB")
print(f"RSS at end:         {rss_end / 1024:.1f} MB")
print(f"CPU used:           {cpu_end:.2f} CPU-s")
print(f"settled after:      {'%.2f s' % settle if settle is not None else 'never'}")
print(f"idle CPU (last {IDLE_WINDOW:.0f} s): {idle:.3f} CPU-s per second")
PY
