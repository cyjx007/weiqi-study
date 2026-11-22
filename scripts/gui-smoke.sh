#!/usr/bin/env bash
# weiqi-gui 冒烟测试：在 Xvfb（或当前 DISPLAY）里启动窗口，按导航键，
# 确认进程不退出、不 panic、不空转。CI 里跑的就是这个脚本。
#
#   scripts/gui-smoke.sh                      # 自动找 target/{release,debug}/weiqi-gui
#   WEIQI_GUI_BIN=target/release/weiqi-gui scripts/gui-smoke.sh
#   scripts/gui-smoke.sh --seconds 10         # 启动后多等一会
#   scripts/gui-smoke.sh --switch             # 额外尝试切换棋谱（需要 xdotool，尽力而为）
#
# 依赖：没有 DISPLAY 时需要 Xvfb；按键和 CPU 检查需要 xdotool（缺了就跳过）。
set -euo pipefail

SECONDS_UP=6
WANT_SWITCH=0
BIN="${WEIQI_GUI_BIN:-}"

while [ $# -gt 0 ]; do
    case "$1" in
        --seconds) SECONDS_UP="$2"; shift 2 ;;
        --binary) BIN="$2"; shift 2 ;;
        --switch) WANT_SWITCH=1; shift ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "未知参数：$1" >&2; exit 2 ;;
    esac
done

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

if [ -z "$BIN" ]; then
    for cand in target/release/weiqi-gui target/debug/weiqi-gui; do
        if [ -x "$cand" ]; then BIN="$cand"; break; fi
    done
fi
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
    echo "找不到 weiqi-gui，先 cargo build -p weiqi-gui 或设 WEIQI_GUI_BIN" >&2
    exit 2
fi
BIN=$(readlink -f "$BIN")

TMP=$(mktemp -d "${TMPDIR:-/tmp}/weiqi-gui-smoke.XXXXXX")
APP_PID=""
XVFB_PID=""
DBUS_PID=""

cleanup() {
    if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
        kill "$APP_PID" 2>/dev/null || true
        for _ in $(seq 20); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
        kill -9 "$APP_PID" 2>/dev/null || true
    fi
    if [ -n "$XVFB_PID" ]; then kill "$XVFB_PID" 2>/dev/null || true; fi
    if [ -n "$DBUS_PID" ]; then kill "$DBUS_PID" 2>/dev/null || true; fi
    rm -rf "$TMP"
}
trap cleanup EXIT

fail() {
    echo "FAIL: $*" >&2
    echo "--- gui.log ---" >&2
    tail -n 40 "$TMP/gui.log" 2>/dev/null >&2 || true
    exit 1
}

mkdir -p "$TMP/data/weiqi-study/games"
printf '(;GM[1]FF[4]CA[UTF-8]SZ[19]KM[7.5]RU[Chinese];B[cc];W[gg])\n' \
    > "$TMP/data/weiqi-study/games/game-101.sgf"
printf '(;GM[1]FF[4]CA[UTF-8]SZ[19]KM[7.5]RU[Chinese];B[cc];W[gg];B[dd];W[ee];B[ff];W[hh])\n' \
    > "$TMP/data/weiqi-study/games/game-102.sgf"

if [ -z "${DISPLAY:-}" ]; then
    if ! command -v Xvfb >/dev/null; then
        echo "没有 DISPLAY 也没有 Xvfb；跳过 GUI 冒烟" >&2
        exit 2
    fi
    export DISPLAY=:${WEIQI_SMOKE_DISPLAY:-99}
    Xvfb "$DISPLAY" -screen 0 1280x900x24 >"$TMP/xvfb.log" 2>&1 &
    XVFB_PID=$!
    for _ in $(seq 50); do
        [ -S "/tmp/.X11-unix/X${DISPLAY#:}" ] && break
        sleep 0.1
    done
fi

export GDK_BACKEND=x11
export XDG_DATA_HOME="$TMP/data"
export WEIQI_LANG=zh

# 私有一个 D-Bus 会话：adw::Application 按 application-id 做单实例，
# 共用会话总线时第二次启动会直接把激活转给已有窗口然后退出（CI 里也可能有残留实例）。
if command -v dbus-daemon >/dev/null; then
    BUS_INFO=$(dbus-daemon --session --fork --print-address=1 --print-pid=1)
    export DBUS_SESSION_BUS_ADDRESS
    DBUS_SESSION_BUS_ADDRESS=$(printf '%s\n' "$BUS_INFO" | sed -n 1p)
    DBUS_PID=$(printf '%s\n' "$BUS_INFO" | sed -n 2p)
fi

"$BIN" >"$TMP/gui.log" 2>&1 &
APP_PID=$!

HAVE_XDOTOOL=0
if command -v xdotool >/dev/null; then HAVE_XDOTOOL=1; fi

WID=""
if [ "$HAVE_XDOTOOL" = 1 ]; then
    for _ in $(seq 100); do
        WID=$(xdotool search --name 'Weiqi Study' 2>/dev/null | head -1 || true)
        [ -n "$WID" ] && break
        kill -0 "$APP_PID" 2>/dev/null || break
        sleep 0.1
    done
fi

sleep "$SECONDS_UP"

if ! kill -0 "$APP_PID" 2>/dev/null; then
    fail "启动 ${SECONDS_UP}s 后进程已退出"
fi
if grep -qE 'panicked at|thread .* panicked|SIGABRT|Aborted' "$TMP/gui.log"; then
    fail "日志里有 panic"
fi
if [ "$HAVE_XDOTOOL" = 1 ] && [ -z "$WID" ]; then
    fail "找不到名为 'Weiqi Study' 的窗口"
fi

cpu_percent() {
    local pid=$1 a b hz
    a=$(awk '{print $14+$15}' "/proc/$pid/stat")
    sleep 2
    b=$(awk '{print $14+$15}' "/proc/$pid/stat")
    hz=$(getconf CLK_TCK)
    echo $(( (b - a) * 100 / (2 * hz) ))
}

if [ -n "$WID" ]; then
    xdotool windowactivate "$WID" 2>/dev/null \
        || xdotool windowfocus "$WID" 2>/dev/null || true
    # 键盘导航：下一步/上一步/开局/终局/退出预览
    for k in n n p g G Escape; do
        xdotool key --window "$WID" "$k" >/dev/null 2>&1 || true
        sleep 0.2
    done
    kill -0 "$APP_PID" 2>/dev/null || fail "键盘导航后进程退出"

    if [ "$WANT_SWITCH" = 1 ]; then
        # 尽力而为：Tab 到棋谱下拉框（prev/next 之后的第三个控件）再切换。
        xdotool key --window "$WID" Tab Tab Tab >/dev/null 2>&1 || true
        sleep 0.3
        xdotool key --window "$WID" Down >/dev/null 2>&1 || true
        sleep 0.3
        xdotool key --window "$WID" Down Return >/dev/null 2>&1 || true
        sleep 1
        kill -0 "$APP_PID" 2>/dev/null || fail "切换棋谱后进程退出"
    fi

    CPU=$(cpu_percent "$APP_PID")
    if [ "$CPU" -gt 80 ]; then
        fail "空闲 CPU ${CPU}%（疑似空转）"
    fi
    echo "窗口: $WID  CPU: ${CPU}%"
else
    echo "未安装 xdotool，只检查启动"
fi

kill "$APP_PID" 2>/dev/null || true
for _ in $(seq 30); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$APP_PID" 2>/dev/null; then
    fail "收到 SIGTERM 后 3s 仍未退出"
fi
APP_PID=""

echo "PASS: $BIN"
