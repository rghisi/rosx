#!/usr/bin/env bash
set -u

base_branch="${1:-${RETRO_BASE:-main}}"
repeat_edit_threshold=3

base_ref=""
for candidate in "origin/${base_branch}" "${base_branch}"; do
  if git rev-parse --verify --quiet "${candidate}" >/dev/null; then
    base_ref="${candidate}"
    break
  fi
done
if [ -z "${base_ref}" ]; then
  echo "signals: no base branch '${base_branch}' found" >&2
  exit 1
fi

merge_base="$(git merge-base "${base_ref}" HEAD)"
range="${merge_base}..HEAD"

echo "## branch"
echo "branch: $(git rev-parse --abbrev-ref HEAD)"
echo "base: ${base_ref} @ $(git rev-parse --short "${merge_base}")"
echo "commits: $(git rev-list --count "${range}")"
echo "uncommitted paths: $(git status --porcelain | wc -l | tr -d ' ')"
echo

echo "## revert commits"
git log --format='%h %s' --grep='^Revert' "${range}" | sed 's/^/  /'
echo

echo "## fix-shaped commits (heuristic: subject contains fix|fixup|squash|oops|typo)"
git log --format='%h %s' -i -E --grep='\b(fix|fixup|squash|oops|typo)' "${range}" | sed 's/^/  /'
echo

echo "## files touched in >= ${repeat_edit_threshold} commits"
git log --name-only --format= "${range}" \
  | grep -v -e '^$' -e '^Cargo.lock$' | sort | uniq -c | sort -rn \
  | awk -v min="${repeat_edit_threshold}" '$1 >= min {print "  " $0}'
echo

echo "## diff size"
git diff --shortstat "${merge_base}" HEAD | sed 's/^/  /'
echo

echo "## pull request"
if ! command -v gh >/dev/null 2>&1; then
  echo "  gh not installed"
  exit 0
fi
pr_number="$(gh pr view --json number --jq .number 2>/dev/null || true)"
if [ -z "${pr_number}" ]; then
  echo "  no open pull request for this branch"
  exit 0
fi
gh pr view "${pr_number}" --json number,state,reviewDecision \
  --jq '"  #\(.number) state=\(.state) review=\(.reviewDecision // "none")"' 2>/dev/null \
  || echo "  pr view failed"
echo "  check conclusions:"
gh pr view "${pr_number}" --json statusCheckRollup \
  --jq '.statusCheckRollup[]? | "    \(.name // .context): \(.conclusion // .state // "pending")"' 2>/dev/null \
  || echo "    unavailable"
echo "  review comments: $(gh api "repos/{owner}/{repo}/pulls/${pr_number}/comments" --jq length 2>/dev/null || echo unavailable)"
echo "  conversation comments: $(gh api "repos/{owner}/{repo}/issues/${pr_number}/comments" --jq length 2>/dev/null || echo unavailable)"
