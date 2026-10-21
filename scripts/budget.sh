#!/usr/bin/env bash
# Resource budget: sample the daemon (`clashofmeat daemon`) and the desktop app (`clashofmeat-desktop`) with ps over a
# window and fail when either is over budget. See docs/building.md, "Resource budget".
#
# Design target: the daemon idles at about 10 MB RSS and about 0% CPU; the overlay redraws only while the hero moves.
# Limits, with headroom over what was measured on Paul's Mac (2026-10-21, Apple Silicon, 90-120 s windows):
#   release daemon, idle                 4-7 MB RSS, 0.07-0.09% CPU
#   debug daemon, agents writing logs    26.5 MB RSS, 1.1% CPU (it parses provider logs 300 ms after each change)
#   release app, hero shown              84 MB RSS, 0.42% CPU
#   debug app, hero shown                44 MB RSS, 1.6% CPU
#   -> daemon 25 MB RSS / 1% average CPU; app 150 MB RSS / 3% average CPU.
# The limits are for release builds with no agent session writing logs. The daemon limit is ~4x its idle RSS and
# ~10x its idle CPU, so parsing a burst of logs fits but a leak or a busy loop does not. The app limit leaves ~75%
# headroom over the release app: a moving hero redraws, and 3% average still catches a render loop that never stops.
# The WebView's WebKit helper processes (WebContent, GPU, Networking) run under launchd, not under
# the app, so ps cannot attribute them and they are not counted.
#
# Average CPU is cumulative CPU time used during the window divided by the window length (not ps %cpu, which is a
# decaying average). RSS is the largest sample.
set -euo pipefail

die() { echo "budget: $*" >&2; exit 2; }
usage="usage: $0 [--window SECONDS] [--interval SECONDS] [--daemon-pid PID] [--app-pid PID] [--only daemon|app]
       [--daemon-rss-mb N] [--daemon-cpu PCT] [--app-rss-mb N] [--app-cpu PCT]"

window=60
interval=5
daemon_pid=
app_pid=
only=
daemon_rss_mb=25
daemon_cpu=1
app_rss_mb=150
app_cpu=3
while (( $# )); do
  case "$1" in
    --window|--interval|--daemon-pid|--app-pid|--only|--daemon-rss-mb|--daemon-cpu|--app-rss-mb|--app-cpu)
      [[ $# -ge 2 && -n $2 ]] || die "$usage"
      name=${1#--}
      printf -v "${name//-/_}" '%s' "$2"
      shift 2 ;;
    -h|--help) echo "$usage"; exit 0 ;;
    *) die "$usage" ;;
  esac
done
[[ $window =~ ^[0-9]+$ && $window -ge 1 ]] || die "--window must be a whole number of seconds"
[[ $interval =~ ^[0-9]+$ && $interval -ge 1 ]] || die "--interval must be a whole number of seconds"
case "$only" in ''|daemon|app) ;; *) die "--only must be daemon or app" ;; esac

# Find one process by a ps command pattern; refuse to guess when several match.
find_pid() {
  local label=$1 pattern=$2 matches
  matches=$(ps -axo pid=,command= | awk -v pattern="$pattern" '$0 ~ pattern { print $1 }')
  case "$(printf '%s' "$matches" | grep -c .)" in
    0) die "no running $label found; start it or pass --$label-pid" ;;
    1) printf '%s\n' "$matches" ;;
    *) die "several $label processes ($(echo $matches)); pass --$label-pid" ;;
  esac
}

labels=()
pids=()
if [[ $only != app ]]; then
  [[ -n $daemon_pid ]] || daemon_pid=$(find_pid daemon '^ *[0-9]+ ([^ ]*/)?clashofmeat daemon( |$)')
  labels+=(daemon); pids+=("$daemon_pid")
fi
if [[ $only != daemon ]]; then
  [[ -n $app_pid ]] || app_pid=$(find_pid app '^ *[0-9]+ .*/clashofmeat-desktop( |$)')
  labels+=(app); pids+=("$app_pid")
fi
for i in "${!pids[@]}"; do
  [[ ${pids[$i]} =~ ^[0-9]+$ ]] && ps -p "${pids[$i]}" >/dev/null || die "${labels[$i]} pid ${pids[$i]} is not running"
done

# ps prints CPU time as [[dd-]hh:]mm:ss.cc; print it in seconds.
cpu_seconds() {
  ps -o time= -p "$1" | awk '{
    t = $1; days = 0
    if (index(t, "-")) { split(t, d, "-"); days = d[1]; t = d[2] }
    n = split(t, p, ":"); s = 0
    for (i = 1; i <= n; i++) s = s * 60 + p[i]
    printf "%.2f\n", s + days * 86400
  }'
}
rss_kb() { ps -o rss= -p "$1" | tr -d ' '; }

declare -a cpu_start max_rss
for i in "${!pids[@]}"; do
  cpu_start[$i]=$(cpu_seconds "${pids[$i]}")
  max_rss[$i]=$(rss_kb "${pids[$i]}")
  echo "budget: ${labels[$i]} pid ${pids[$i]}: $(ps -o command= -p "${pids[$i]}")"
done
echo "budget: sampling for ${window}s every ${interval}s"
started=$(date +%s)
elapsed=0
while (( elapsed < window )); do
  step=$(( window - elapsed < interval ? window - elapsed : interval ))
  sleep "$step"
  for i in "${!pids[@]}"; do
    ps -p "${pids[$i]}" >/dev/null || die "${labels[$i]} pid ${pids[$i]} exited during the window"
    rss=$(rss_kb "${pids[$i]}")
    if (( rss > max_rss[i] )); then max_rss[$i]=$rss; fi
  done
  elapsed=$(( $(date +%s) - started ))
done

failed=0
for i in "${!pids[@]}"; do
  label=${labels[$i]}
  if [[ $label == daemon ]]; then rss_limit=$daemon_rss_mb; cpu_limit=$daemon_cpu; else rss_limit=$app_rss_mb; cpu_limit=$app_cpu; fi
  cpu_end=$(cpu_seconds "${pids[$i]}")
  read -r verdict line < <(awk -v label="$label" -v pid="${pids[$i]}" -v start="${cpu_start[$i]}" -v end="$cpu_end" \
    -v wall="$elapsed" -v rss="${max_rss[$i]}" -v rss_limit="$rss_limit" -v cpu_limit="$cpu_limit" 'BEGIN {
      cpu = (end - start) / wall * 100; mb = rss / 1024
      ok = (mb <= rss_limit && cpu <= cpu_limit) ? "ok" : "over"
      printf "%s %-6s pid %-6s max RSS %6.1f MB (limit %s)  avg CPU %5.2f%% (limit %s%%)  %s\n",
        ok, label, pid, mb, rss_limit, cpu, cpu_limit, toupper(ok == "ok" ? "ok" : "over budget")
    }')
  echo "budget: $line"
  [[ $verdict == ok ]] || failed=1
done
exit "$failed"
