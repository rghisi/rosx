#!/usr/bin/env bash
set -u

if [ $# -lt 1 ]; then
  echo "usage: recurrence.sh <CAUSE-CLASS> [keyword]" >&2
  exit 2
fi

cause_class="$1"
keyword="${2:-}"
plans_dir="${PLANS_DIR:-plans}"

matches="$(
  for plan in "${plans_dir}"/*-impl.md; do
    [ -e "${plan}" ] || continue
    awk -v file="${plan}" -v cls="${cause_class}" -v kw="${keyword}" '
      /^## Friction log/ { in_log = 1; next }
      /^## /            { in_log = 0 }
      in_log && index($0, "|") {
        n = split($0, cols, "|")
        gsub(/^ +| +$/, "", cols[3])
        if (cols[3] == cls && (kw == "" || index(tolower($0), tolower(kw)))) {
          print file ":" NR ": " $0
        }
      }
    ' "${plan}"
  done
)"

if [ -z "${matches}" ]; then
  echo "plans with a matching entry: 0"
  exit 0
fi

echo "${matches}"
echo "plans with a matching entry: $(echo "${matches}" | cut -d: -f1 | sort -u | wc -l | tr -d ' ')"
