#!/usr/bin/env python3
"""TUI 图片棋盘协议冒烟：伪终端里强制 kitty / sixel，检查真的输出了图形转义序列。

这不是“真实终端人工目测”：伪终端不会真的显示图片，只验证
`--mode image` 走到了对应编码器（kitty `ESC _G` / sixel DCS），
并且进程不 panic、能正常退出。真实观感仍需在 kitty / foot 里跑一次。
"""

from __future__ import annotations

import argparse
import fcntl
import os
import pty
import select
import shutil
import signal
import struct
import sys
import tempfile
import termios
import time

ROWS, COLS = 40, 120

# (名字, 额外的环境变量, 输出里必须出现的协议标记)
CASES: list[tuple[str, dict[str, str], bytes]] = [
    (
        "kitty",
        {"TERM": "xterm-kitty", "KITTY_WINDOW_ID": "1"},
        b"\x1b_G",  # kitty graphics protocol
    ),
    (
        "sixel",
        {"TERM": "xterm-sixel"},
        b"\x1bP",  # DCS，sixel 编码以 ESC P ... q 开头
    ),
]


def find_binary(explicit: str | None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("WEIQI_TUI_BIN")
    if env:
        return env
    for candidate in ("target/release/weiqi-tui", "target/debug/weiqi-tui"):
        if os.access(candidate, os.X_OK):
            return candidate
    found = shutil.which("weiqi-tui")
    if found:
        return found
    print("FAIL: 找不到 weiqi-tui（设 WEIQI_TUI_BIN）", file=sys.stderr)
    raise SystemExit(2)


def read_until_quiet(fd: int, out: bytearray, seconds: float) -> None:
    deadline = time.time() + seconds
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.05)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            return
        if not chunk:
            return
        if len(out) < 8_000_000:
            out.extend(chunk)


def run_case(binary: str, sgf: str, workdir: str, case) -> bool:
    name, extra_env, marker = case
    env = os.environ.copy()
    # 外层终端可能就是 kitty：先清掉会抢先命中 kitty/iTerm2 的变量
    for key in (
        "KITTY_WINDOW_ID",
        "KITTY_PID",
        "KITTY_INSTALLATION_DIR",
        "WEZTERM_EXECUTABLE",
        "ITERM_SESSION_ID",
        "TERM_PROGRAM",
    ):
        env.pop(key, None)
    env.update(extra_env)
    env["XDG_DATA_HOME"] = os.path.join(workdir, "data")
    env["KATAGO_BIN"] = "/nonexistent/katago"
    env["KATAGO_MODEL"] = "/nonexistent/model.bin.gz"
    env["KATAGO_CFG"] = "/nonexistent/analysis.cfg"
    env["WEIQI_LANG"] = "en"

    pid, fd = pty.fork()
    if pid == 0:
        try:
            os.execvpe(binary, [binary, sgf, "--mode", "image"], env)
        finally:
            os._exit(127)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    out = bytearray()
    try:
        # 图片模式会等 2s 的终端查询；大图首次编码也要时间
        read_until_quiet(fd, out, 6.0)
        os.write(fd, b"q")
        read_until_quiet(fd, out, 2.0)
        os.write(fd, b"q")
        for _ in range(60):
            done, wstatus = os.waitpid(pid, os.WNOHANG)
            if done:
                break
            time.sleep(0.05)
        else:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            print(f"FAIL[{name}]: 4s 后仍未退出", file=sys.stderr)
            return False
    finally:
        try:
            os.close(fd)
        except OSError:
            pass

    code = os.waitstatus_to_exitcode(wstatus)
    text = bytes(out)
    if code != 0:
        print(f"FAIL[{name}]: 退出码 {code}\n{text[-2000:]!r}", file=sys.stderr)
        return False
    if b"panicked at" in text:
        print(f"FAIL[{name}]: 输出里有 panic", file=sys.stderr)
        return False
    if marker not in text:
        hint = "kitty 会输出 ESC _G" if name == "kitty" else "sixel 会输出 ESC P ... q"
        print(
            f"FAIL[{name}]: 没有找到图形协议序列（{hint}）；"
            f"输出 {len(text)} bytes，可能退回了文本棋盘",
            file=sys.stderr,
        )
        return False
    print(f"PASS[{name}]: 编码器输出了 {marker!r}（{len(text)} bytes）")
    return True


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", help="weiqi-tui 路径（默认 WEIQI_TUI_BIN 或 target/*）")
    args = ap.parse_args()
    binary = find_binary(args.binary)

    workdir = tempfile.mkdtemp(prefix="weiqi-tui-image-smoke-")
    sgf = os.path.join(workdir, "smoke.sgf")
    with open(sgf, "w", encoding="utf-8") as f:
        f.write("(;GM[1]FF[4]SZ[9]KM[7.5]RU[Chinese];B[cc];W[gg];B[dd])\n")
    try:
        ok = True
        for case in CASES:
            ok = run_case(binary, sgf, workdir, case) and ok
        return 0 if ok else 1
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
