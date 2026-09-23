#!/usr/bin/env python3
import argparse
import json
import os
import re
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
import shutil

PROMPT = "rose> "
ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
KEY_MAP = {
    'a':'a','b':'b','c':'c','d':'d','e':'e','f':'f','g':'g','h':'h','i':'i','j':'j','k':'k','l':'l','m':'m','n':'n','o':'o','p':'p','q':'q','r':'r','s':'s','t':'t','u':'u','v':'v','w':'w','x':'x','y':'y','z':'z',
    'A':['shift','a'],'B':['shift','b'],'C':['shift','c'],'D':['shift','d'],'E':['shift','e'],'F':['shift','f'],'G':['shift','g'],'H':['shift','h'],'I':['shift','i'],'J':['shift','j'],'K':['shift','k'],'L':['shift','l'],'M':['shift','m'],'N':['shift','n'],'O':['shift','o'],'P':['shift','p'],'Q':['shift','q'],'R':['shift','r'],'S':['shift','s'],'T':['shift','t'],'U':['shift','u'],'V':['shift','v'],'W':['shift','w'],'X':['shift','x'],'Y':['shift','y'],'Z':['shift','z'],
    '0':'0','1':'1','2':'2','3':'3','4':'4','5':'5','6':'6','7':'7','8':'8','9':'9',
    ' ': 'space',
    '\n': 'ret',
    '\x08': 'backspace',
    '-':'minus',
    '=':'equal',
    ',':'comma',
    '.':'dot',
    '/':'slash',
    ';':'semi_colon',
    "'":'apostrophe',
    '\\':'backslash',
    '[':'left_bracket',
    ']':'right_bracket',
    '`':'grave_accent',
}
class QmpError(Exception):
    pass
class PromptTimeout(Exception):
    def __init__(self, raw, partial=None):
        super().__init__("prompt timeout")
        self.raw = raw
        self.partial = partial

class QmpClient:
    def __init__(self, socket_path):
        self.socket_path = socket_path
        self.sock = None
        self.file = None
    def connect(self, timeout):
        deadline = time.time()+timeout
        while time.time()<deadline:
            try:
                self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.sock.connect(self.socket_path)
                self.file = self.sock.makefile('rwb', buffering=1)
                break
            except FileNotFoundError:
                time.sleep(0.05)
                continue
            except Exception:
                time.sleep(0.05)
                continue
        else:
            raise QmpError("qmp socket connect timeout")
        line = self.file.readline()
        while line:
            obj = json.loads(line.decode())
            if 'QMP' in obj:
                break
            line = self.file.readline()
        self._send({"execute":"qmp_capabilities"})
        resp = self._recv()
        if 'error' in resp:
            raise QmpError(resp['error'])
    def _send(self, obj):
        data = (json.dumps(obj)+"\n").encode()
        self.file.write(data)
        self.file.flush()
    def _recv(self):
        line = self.file.readline()
        while line:
            obj = json.loads(line.decode())
            if 'return' in obj or 'error' in obj:
                return obj
            line = self.file.readline()
        raise QmpError("qmp recv eof")
    def send_key(self, *keys):
        self._send({"execute":"send-key","arguments":{"key":list(keys)}})
        resp = self._recv()
        if 'error' in resp:
            raise QmpError(resp['error'])
    def close(self):
        try:
            if self.file:
                self.file.close()
            if self.sock:
                self.sock.close()
        except Exception:
            pass

class OutputBuffer:
    def __init__(self):
        self.lock = threading.Lock()
        self.cond = threading.Condition(self.lock)
        self.buf = ""
    def append(self, text):
        with self.cond:
            self.buf += text
            self.cond.notify_all()
    def wait_for(self, needle, timeout):
        deadline = time.time()+timeout
        with self.cond:
            idx = self.buf.rfind(needle)
            while idx==-1:
                remaining = deadline-time.time()
                if remaining<=0:
                    raise PromptTimeout(self.buf)
                self.cond.wait(timeout=remaining)
                idx = self.buf.rfind(needle)
            return idx+len(needle)
    def text_from(self, index):
        with self.lock:
            return self.buf[index:]
    def snapshot(self):
        with self.lock:
            return self.buf

class RosxShell:
    def __init__(self):
        self.proc = None
        self.buffer = OutputBuffer()
        self.qmp = None
        self.reader = None
        self.qmp_socket = None
    def start(self, kernel=None, image=None, qmp_socket="target/rosx/qmp.sock", boot_timeout=120.0, key_delay_ms=20):
        if shutil.which("qemu-system-x86_64") is None:
            raise RuntimeError("qemu-system-x86_64 not found")
        self.qmp_socket = qmp_socket
        try:
            os.unlink(qmp_socket)
        except FileNotFoundError:
            pass
        if image:
            cmd = ["qemu-system-x86_64","-drive",f"format=raw,file={image}","-debugcon","stdio","-no-reboot","-no-shutdown","-d","cpu_reset","-qmp",f"unix={qmp_socket},server,nowait","-display","none"]
            stderr_path = qmp_socket+".log"
        else:
            runner = Path("arch/x86_64-runner/target/debug/runner")
            if not runner.exists():
                subprocess.run(["cargo","build","--manifest-path","arch/x86_64-runner/Cargo.toml"], check=True)
            kernel_path = kernel or "target/rosx/debug/rosx"
            cmd = [str(runner), kernel_path, "x86_64","--qmp", qmp_socket]
            stderr_path = qmp_socket+".log"
        self.proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=open(stderr_path,"wb"), start_new_session=True)
        self.reader = threading.Thread(target=self._read_loop, daemon=True)
        self.reader.start()
        self.qmp = QmpClient(qmp_socket)
        self.qmp.connect(boot_timeout)
        self.buffer.wait_for(PROMPT, boot_timeout)
        self.key_delay_ms = key_delay_ms
    def _read_loop(self):
        try:
            while True:
                data = os.read(self.proc.stdout.fileno(), 4096)
                if not data:
                    break
                text = data.decode('latin-1', 'ignore')
                clean = ANSI_RE.sub('', text).replace('\r','')
                self.buffer.append(clean)
        except Exception:
            pass
    def type_text(self, s):
        for ch in s:
            if ch in KEY_MAP:
                val = KEY_MAP[ch]
                if isinstance(val, list):
                    self.qmp.send_key(*val)
                else:
                    self.qmp.send_key(val)
            else:
                raise ValueError(f"unmapped char {repr(ch)}")
            time.sleep(self.key_delay_ms/1000.0)
    def command(self, cmd, timeout=30.0):
        idx = self.buffer.wait_for(PROMPT, timeout)
        mark = idx
        self.type_text(cmd+"\n")
        try:
            next_idx = self.buffer.wait_for(PROMPT, timeout)
        except PromptTimeout as e:
            raise PromptTimeout(self.buffer.snapshot(), self.buffer.text_from(mark))
        region = self.buffer.text_from(mark)
        return region[:region.find(PROMPT)]
    def close(self):
        try:
            if self.proc and self.proc.poll() is None:
                try:
                    os.killpg(os.getpgid(self.proc.pid), signal.SIGTERM)
                except Exception:
                    self.proc.terminate()
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(os.getpgid(self.proc.pid), signal.SIGKILL)
                    except Exception:
                        self.proc.kill()
        except Exception:
            pass
        try:
            if self.qmp:
                self.qmp.close()
        except Exception:
            pass
        try:
            if self.qmp_socket:
                os.unlink(self.qmp_socket)
        except Exception:
            pass
    def __enter__(self):
        return self
    def __exit__(self, exc_type, exc, tb):
        self.close()

def main(argv=None):
    parser = argparse.ArgumentParser(description="RosX QEMU automation agent")
    parser.add_argument("--kernel", default=None)
    parser.add_argument("--image")
    parser.add_argument("--qmp-socket", default="target/rosx/qmp.sock")
    parser.add_argument("--boot-timeout", type=float, default=120.0)
    parser.add_argument("--key-delay-ms", type=int, default=20)
    parser.add_argument("--repl", action="store_true")
    parser.add_argument("--check", action="append", default=[])
    parser.add_argument("--check-file")
    parser.add_argument("--dump", action="store_true")
    parser.add_argument("--dump-seconds", type=int, default=8)
    args = parser.parse_args(argv)
    if args.image and args.kernel:
        print("use --image or --kernel", file=sys.stderr)
        return 2
    try:
        vm = RosxShell()
        vm.start(kernel=args.kernel, image=args.image, qmp_socket=args.qmp_socket, boot_timeout=args.boot_timeout, key_delay_ms=args.key_delay_ms)
    except Exception as e:
        print(f"setup error: {e}", file=sys.stderr)
        return 2
    try:
        checks = []
        if args.check:
            checks.extend(args.check)
        if args.check_file:
            with open(args.check_file) as f:
                for line in f:
                    line=line.strip()
                    if not line or line.startswith('#'):
                        continue
                    checks.append(line)
        if args.dump:
            start = time.time()
            while time.time()-start < args.dump_seconds:
                time.sleep(0.1)
            print(vm.buffer.snapshot())
            return 0
        if args.repl:
            while True:
                try:
                    cmd = input("rosx> ")
                except EOFError:
                    break
                if cmd.strip()=="quit":
                    break
                out = vm.command(cmd, timeout=30)
                print(out)
            return 0
        if checks:
            ok=True
            for spec in checks:
                if '|' not in spec:
                    print(f"FAIL {spec}: bad spec")
                    ok=False
                    continue
                cmd, expect = spec.split('|',1)
                try:
                    out = vm.command(cmd, timeout=30)
                    if expect and expect not in out:
                        print(f"FAIL {spec}: expectation not found")
                        ok=False
                    else:
                        print(f"PASS {spec}")
                except PromptTimeout as e:
                    print(f"FAIL {spec}: timeout")
                    ok=False
                except Exception as e:
                    print(f"FAIL {spec}: {e}")
                    ok=False
            return 0 if ok else 1
        return 0
    finally:
        vm.close()
if __name__=="__main__":
    sys.exit(main())
