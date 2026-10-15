---
description: Create a new git branch for a theme, based on an up-to-date main.
---

Set up a new git branch for this theme:

$ARGUMENTS

Steps, in strict order:

1. Derive a branch name from the theme: lowercase, words separated by single hyphens, strip anything that is not `[a-z0-9/-]` (e.g. "Extract IPC handles" -> `extract-ipc-handles`).
2. Run `git status --porcelain`. If there are uncommitted changes, STOP and report them to the user; do not switch branches.
3. Run `git fetch origin`.
4. Run `git checkout main` (fall back to `master` if `main` does not exist).
5. Run `git pull origin <default-branch>` to bring it up to date with origin.
6. Run `git checkout -b <branch-name>`. If the branch already exists locally, STOP and ask the user whether to switch to it or pick another name.
7. Report the new branch name and the commit it points at (`git log -1 --oneline`).

Do not modify any files. Git operations only.
