#!/bin/bash
# usage: eval.sh <file.rs> [sims]   -> prints in-sample (seeds 0..) + out-of-sample (seeds 100000..) avg edge
cd ~/prop-amm-challenge || exit 1
f="$1"; sims="${2:-1000}"
bin=./target/release/prop-amm
o1=$("$bin" run "$f" --simulations "$sims" 2>&1)
is=$(echo "$o1" | sed -nE 's/.*Avg edge:[[:space:]]*([0-9.\-]+).*/\1/p')
if [ -z "$is" ]; then
  echo "$(basename "$f")  BUILD/RUN FAILED:"
  echo "$o1" | grep -iE 'error\[|error:|panic|violation' | head -6
  exit 0
fi
o2=$("$bin" run "$f" --simulations "$sims" --seed-start 100000 --seed-stride 1 2>/dev/null)
oos=$(echo "$o2" | sed -nE 's/.*Avg edge:[[:space:]]*([0-9.\-]+).*/\1/p')
printf "%-26s in=%-9s oos=%-9s\n" "$(basename "$f")" "$is" "${oos:-NA}"
