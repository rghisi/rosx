---
description: Restart the running opencode TUI in place via tmux, resuming the current session.
---

Restart the opencode instance that is running this command, in the same terminal pane.

1. Check the environment first:

```bash
echo "TMUX=$TMUX TMUX_PANE=$TMUX_PANE"
```

If `TMUX_PANE` is empty, STOP. Report to the user that this opencode session was not started inside tmux, so an in-place restart is not possible, and suggest relaunching opencode as `tmux new -s dev 'opencode'` (or interactively in tmux) and using /restart again there.

2. Otherwise run exactly this single command, nothing else:

```bash
tmux respawn-pane -k -t "$TMUX_PANE" "cd '$PWD' && $HOME/.config/opencode/launch.sh --continue"
```

This tells the tmux server (which survives the kill) to terminate this pane's process and relaunch opencode in the same pane and directory, resuming the last session.

3. The command kills opencode while executing, so the tool call will appear interrupted. That is the expected outcome. Never run any command after it, and treat "did not return" as success.
