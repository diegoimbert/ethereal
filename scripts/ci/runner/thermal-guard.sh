#!/usr/bin/env bash
# Thermal guard of the CI slices (docs/CI-RUNNERS.md), run every 20 s by ethereal-ci-thermal.timer
# (`pool.mjs setup` installs both). When the CPU (k10temp Tctl) or the hottest DIMM (spd5118) runs
# hot, it lowers the CPU quota of ethereal-ci.slice and ethereal-ci-pool.slice; when both have
# cooled down (hysteresis), it gives the quotas back. Running jobs keep going, only slower.
#
# Settings (environment, from ~/.config/ethereal-ci/env through the unit):
#   CI_CPUS                 quota of ethereal-ci.slice, all CI      (CI_POOL_CPUS)
#   CI_POOL_CPUS            quota of ethereal-ci-pool.slice         (8)
#   CI_THERMAL_LOW_CPUS     quota of both slices while hot          (half of CI_CPUS)
#   CI_THERMAL_HOT_TCTL     hot at or above, C                      (90)
#   CI_THERMAL_HOT_DIMM                                             (75)
#   CI_THERMAL_COOL_TCTL    cool again at or below (both), C        (80)
#   CI_THERMAL_COOL_DIMM                                            (65)
#   CI_THERMAL_DIR          state and log                           (~/ci-pool/thermal)
#   CI_THERMAL_HWMON        sensors                                 (/sys/class/hwmon)
# A machine without these sensors reads 0 C and is never throttled.
set -u
pool=${CI_POOL_CPUS:-8}
ci=${CI_CPUS:-$pool}
low=${CI_THERMAL_LOW_CPUS:-$((ci / 2))}
hot_tctl=${CI_THERMAL_HOT_TCTL:-90} hot_dimm=${CI_THERMAL_HOT_DIMM:-75}
cool_tctl=${CI_THERMAL_COOL_TCTL:-80} cool_dimm=${CI_THERMAL_COOL_DIMM:-65}
dir=${CI_THERMAL_DIR:-$HOME/ci-pool/thermal}
mkdir -p "$dir"

tctl=0 dimm=0
for h in "${CI_THERMAL_HWMON:-/sys/class/hwmon}"/*; do
  t=$(cat "$h/temp1_input" 2>/dev/null) || continue
  t=$((t / 1000))
  case $(cat "$h/name" 2>/dev/null) in
    k10temp) tctl=$t ;;
    spd5118) [ "$t" -gt "$dimm" ] && dimm=$t ;;
  esac
done

cur=$(cat "$dir/state" 2>/dev/null || echo full)
want=$cur
if [ "$tctl" -ge "$hot_tctl" ] || [ "$dimm" -ge "$hot_dimm" ]; then
  want=low
elif [ "$tctl" -le "$cool_tctl" ] && [ "$dimm" -le "$cool_dimm" ]; then
  want=full
fi
if [ "$want" = low ]; then
  q_ci=$((low < ci ? low : ci)) q_pool=$((low < pool ? low : pool))
else
  q_ci=$ci q_pool=$pool
fi

# Reconcile every run, not only on a change: `pool.mjs setup` or an edited env file may have moved
# the quotas meanwhile. --runtime: a reboot starts from the unit files' quotas.
changed=
for s in "ethereal-ci.slice $q_ci" "ethereal-ci-pool.slice $q_pool"; do
  set -- $s
  have=$(systemctl --user show "$1" -p CPUQuotaPerSecUSec --value 2>/dev/null)
  if [ "$have" != "${2}s" ]; then
    systemctl --user set-property --runtime "$1" "CPUQuota=$(($2 * 100))%" && changed=1
  fi
done
echo "$want" > "$dir/state"
echo "$(date -Is) tctl=${tctl}C dimm=${dimm}C $want ci=$q_ci pool=$q_pool" > "$dir/last"
if [ "$want" != "$cur" ] || [ -n "$changed" ]; then
  echo "$(date -Is) tctl=${tctl}C dimm=${dimm}C -> $want (ci $q_ci CPUs, pool $q_pool)" >> "$dir/log"
fi
