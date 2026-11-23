#!/usr/bin/env python3
"""weiqi-tui 冒烟测试：在真实 pty 里启动 TUI，按键，确认不崩溃且能正常退出。

覆盖：启动渲染 → L 打开棋谱列表 → Esc 关闭 → a 发起分析（引擎故意不可用）
→ ? 帮助 → q 退出。分析失败也必须是“进程活着、状态栏报错”，不能 panic。

用法：
    WEIQI_TUI_BIN=target/release/weiqi-tui python3 scripts/tui-smoke.py
    python3 scripts/tui-smoke.py --binary ./result/bin/weiqi-tui --verbose

为什么用 pty：crossterm 需要 tty，且在 0×0 窗口下 ratatui 会 panic，
所以这里显式把窗口设成 120×40。
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

# (等待秒数, 要发的键)。空 bytes 表示只等待。
SCRIPT: list[tuple[float, bytes]] = [
    (1.0, b""),  # 等首帧
    (0.3, b"L"),  # 棋谱列表
    (0.5, b"\x1b"),  # Esc 关闭
    (0.4, b"a"),  # 发起分析（引擎不可用，应显示错误）
    (0.8, b""),  # 等错误回填
    (0.3, b"?"),  # 帮助
    (0.3, b"?"),  # 关帮助
    (0.3, b"q"),  # 退出
]

ROWS, COLS = 40, 120


def find_binary(explicit: str | None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("WEIQI_TUI_BIN")
    if env:
        return env
    for candidate in ("target/release/weiqi-tui", "target/debug/weiqi-tui"):
        if os.access(candidate, os.X_OK):
            return candidate
    return "weiqi-tui"


def write_fixture(dirpath: str) -> str:
    path = os.path.join(dirpath, "smoke.sgf")
    with open(path, "w", encoding="utf-8") as f:
        f.write("(;GM[1]FF[4]SZ[9]KM[7.5]RU[Chinese];B[cc];W[gg];B[dd])\n")
    return path


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", help="weiqi-tui 路径（默认 WEIQI_TUI_BIN 或 target/*）")
    ap.add_argument("--timeout", type=float, default=15.0, help="整体超时（秒）")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    binary = find_binary(args.binary)
    if not (os.path.isabs(binary) or "/" in binary):
        found = shutil.which(binary)
        if not found:
            print(f"FAIL: 找不到 {binary}", file=sys.stderr)
            return 2
        binary = found

    workdir = tempfile.mkdtemp(prefix="weiqi-tui-smoke-")
    sgf = write_fixture(workdir)
    env = os.environ.copy()
    # 不碰真实棋谱库，也不真的去加载 KataGo 模型
    env["XDG_DATA_HOME"] = os.path.join(workdir, "data")
    env["KATAGO_BIN"] = "/nonexistent/katago"
    env["KATAGO_MODEL"] = "/nonexistent/model.bin.gz"
    env["KATAGO_CFG"] = "/nonexistent/analysis.cfg"
    env["TERM"] = "xterm-256color"
    env["WEIQI_LANG"] = "en"

    pid, fd = pty.fork()
    if pid == 0:  # 子进程
        try:
            os.execvpe(binary, [binary, sgf, "--mode", "text"], env)
        finally:
            os._exit(127)

    # 给 TUI 一个真实窗口尺寸，否则 ratatui 的 area 为 0 会崩
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

    out = bytearray()
    idx = 0
    next_at = time.time() + SCRIPT[0][0]
    deadline = time.time() + args.timeout
    status: int | None = None

    try:
        while True:
            ready, _, _ = select.select([fd], [], [], 0.05)
            if ready:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:  # 子进程退出后读 pty 会 EIO
                    chunk = b""
                if chunk:
                    if len(out) < 4_000_000:
                        out.extend(chunk)

            now = time.time()
            if idx < len(SCRIPT) and now >= next_at:
                if SCRIPT[idx][1]:
                    os.write(fd, SCRIPT[idx][1])
                idx += 1
                if idx < len(SCRIPT):
                    next_at = now + SCRIPT[idx][0]

            done, wstatus = os.waitpid(pid, os.WNOHANG)
            if done:
                status = wstatus
                break
            if now > deadline:
                print(
                    f"FAIL: {args.timeout:.0f}s 内没有退出（可能卡死）",
                    file=sys.stderr,
                )
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
                return 1
    finally:
        try:
            os.close(fd)
        except OSError:
            pass

    text = bytes(out)
    if args.verbose:
        sys.stdout.write(text.decode("utf-8", "replace"))

    code = os.waitstatus_to_exitcode(status)
    if code != 0:
        print(f"FAIL: 退出码 {code}", file=sys.stderr)
        return 1
    if b"panicked at" in text or b"RUST_BACKTRACE" in text:
        print("FAIL: 输出里有 panic", file=sys.stderr)
        return 1
    # 至少要真的画过东西（进入 alternate screen 会写 ESC 序列）
    if b"\x1b[" not in text:
        print("FAIL: 没有任何终端输出（可能根本没渲染）", file=sys.stderr)
        return 1
    # WEIQI_LANG=en + KATAGO_BIN 不存在：这两处必须真的出现过
    missing = []
    # 列表里的文件名会被列宽截断，只认前缀
    if b"game-" not in text:
        missing.append("棋谱列表")
    if b"failed" not in text:
        missing.append("分析失败提示")
    if missing:
        print(f"FAIL: 没看到 {'、'.join(missing)}", file=sys.stderr)
        return 1

    print(f"PASS: {os.path.realpath(binary)}（{len(text)} bytes 输出，正常退出）")
    shutil.rmtree(workdir, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
