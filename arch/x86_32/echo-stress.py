#!/usr/bin/env python3
"""x86_32 keystroke-echo stress gate: boot the multiboot kernel in QEMU, wait
for the shell prompt, inject a 10-key burst over the monitor socket, and
require every key to echo back. Runs BOOTS consecutive VM boots; exit code is
nonzero if any run loses a keystroke. This is the M2 acceptance gate of
plans/x86_32-parity-plan.md and the seed of the M6 integration-test harness."""

import os
import socket
import subprocess
import sys
import tempfile
import threading
import time

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_ROOT = os.path.abspath(os.path.join(SCRIPT_DIR, "..", ".."))
KERNEL = os.environ.get(
    "ROSX_KERNEL_ELF",
    os.path.join(WORKSPACE_ROOT, "target", "rosx-i686", "debug", "rosx-x86"),
)
QEMU = os.environ.get("QEMU_BINARY", "qemu-system-i386")
BOOTS = int(os.environ.get("ECHO_BOOTS", "10"))
BURST = os.environ.get("ECHO_KEYS", "abcdefghij")
KEY_DELAY = float(os.environ.get("ECHO_KEY_DELAY", "0.45"))
BOOT_BUDGET = float(os.environ.get("BOOT_BUDGET", "60"))


class Monitor:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(path)

    def command(self, cmd, settle=0.05):
        self.sock.sendall((cmd + "\n").encode())
        time.sleep(settle)
        self.sock.settimeout(0.2)
        try:
            while self.sock.recv(4096):
                pass
        except (socket.timeout, ConnectionResetError):
            pass

    def close(self):
        try:
            self.command("quit")
        finally:
            self.sock.close()


def token_for(c):
    if c.islower() and c.isascii():
        return c
    if c.isupper() and c.isascii():
        return "shift-" + c.lower()
    if c.isdigit():
        return c
    return {
        "\n": "ret", " ": "spc", "\t": "tab", "-": "minus", "=": "equal",
        ".": "dot", ",": "comma", "/": "slash", ";": "semicolon",
    }.get(c)


def drain(stream, buf, lock):
    fd = stream.fileno()
    while True:
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        with lock:
            buf.extend(chunk)


def snapshot(buf, lock):
    with lock:
        return bytes(buf)


def wait_for(buf, lock, needle, budget):
    deadline = time.time() + budget
    while time.time() < deadline:
        if needle in snapshot(buf, lock):
            return True
        time.sleep(0.05)
    return False


def run_once():
    tmp = tempfile.mkdtemp(prefix="rosx-echo-stress-")
    mon_path = os.path.join(tmp, "monitor.sock")
    proc = subprocess.Popen(
        [QEMU, "-kernel", KERNEL, "-debugcon", "stdio", "-display", "none",
         "-monitor", "unix:{},server=on,wait=off".format(mon_path),
         "-no-reboot", "-no-shutdown", "-serial", "none", "-parallel", "none"],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    )
    buf, lock = bytearray(), threading.Lock()
    threading.Thread(target=drain, args=(proc.stdout, buf, lock), daemon=True).start()
    mon = None
    try:
        if not wait_for(buf, lock, b"rose>", BOOT_BUDGET):
            tail = snapshot(buf, lock).decode(errors="replace")[-500:]
            raise RuntimeError("no prompt within budget; tail:\n" + tail)
        time.sleep(1.0)
        mark = len(snapshot(buf, lock))
        mon = Monitor(mon_path)
        for c in BURST:
            mon.command("sendkey " + token_for(c))
            time.sleep(KEY_DELAY)
        time.sleep(2.0)
        mon.close()
        mon = None
        echoed = snapshot(buf, lock)[mark:].decode(errors="replace")
        missing = [c for c in BURST if c not in echoed]
        return len(BURST) - len(missing), echoed, missing
    finally:
        if mon is not None:
            try:
                mon.close()
            except OSError:
                pass
        if proc.poll() is None:
            try:
                Monitor(mon_path).close()
            except OSError:
                pass
            proc.kill()
            proc.wait()


def main():
    if not os.path.exists(KERNEL):
        print("missing kernel ELF: {}\nbuild it with `cargo xtask build x86_32`".format(KERNEL))
        return 1
    failed = 0
    for run in range(1, BOOTS + 1):
        try:
            hits, echoed, missing = run_once()
        except RuntimeError as err:
            print("run {}/{}: BOOT-FAIL {}".format(run, BOOTS, err))
            failed += 1
            continue
        status = "ok" if not missing else "MISSING {}".format("".join(missing))
        print("run {}/{}: echoed {}/{} ({})".format(run, BOOTS, hits, len(BURST), status))
        if missing:
            failed += 1
    if failed:
        print("FAIL: {}/{} runs lost keystrokes".format(failed, BOOTS))
        return 1
    print("PASS: {} boots x {} keys, all echoed".format(BOOTS, len(BURST)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
