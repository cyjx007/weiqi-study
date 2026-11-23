#!/usr/bin/env bash
# weiqi-gui 冒烟测试：在 Xvfb（或当前 DISPLAY）里启动窗口，按导航键，
# 确认进程不退出、不 panic、不空转。CI 里跑的就是这个脚本。
#
#   scripts/gui-smoke.sh                      # 自动找 target/{release,debug}/weiqi-gui
#   WEIQI_GUI_BIN=target/release/weiqi-gui scripts/gui-smoke.sh
#   scripts/gui-smoke.sh --seconds 10         # 启动后多等一会
#   scripts/gui-smoke.sh --switch             # 额外尝试切换棋谱（需要 xdotool，尽力而为）
#   scripts/gui-smoke.sh --import             # 按 o 走文件对话框导入一个 SGF 并校验落盘
#
# 依赖：没有 DISPLAY 时需要 Xvfb；按键和 CPU 检查需要 xdotool（缺了就跳过）。
set -euo pipefail

SECONDS_UP=6
WANT_SWITCH=0
WANT_IMPORT=0
BIN="${WEIQI_GUI_BIN:-}"

while [ $# -gt 0 ]; do
    case "$1" in
        --seconds) SECONDS_UP="$2"; shift 2 ;;
        --binary) BIN="$2"; shift 2 ;;
        --switch) WANT_SWITCH=1; shift ;;
        --import) WANT_IMPORT=1; shift ;;
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

# 信息类参数必须在 GTK 初始化前就能用（不需要 DISPLAY）
VERSION=$("$BIN" --version 2>/dev/null || true)
case "$VERSION" in
    "weiqi-gui "*) ;;
    *) echo "FAIL: --version 没输出预期内容：$VERSION" >&2; exit 1 ;;
esac

TMP=$(mktemp -d "${TMPDIR:-/tmp}/weiqi-gui-smoke.XXXXXX")
APP_PID=""
XVFB_PID=""
DBUS_PID=""

cleanup() {
    if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
        kill "$APP_PID" 2>/dev/null || true
        for _ in $(seq 20); do alive "$APP_PID" || break; sleep 0.1; done
        kill -9 "$APP_PID" 2>/dev/null || true
    fi
    if [ -n "$XVFB_PID" ]; then kill "$XVFB_PID" 2>/dev/null || true; fi
    if [ -n "$DBUS_PID" ]; then kill "$DBUS_PID" 2>/dev/null || true; fi
    rm -rf "$TMP"
}
trap cleanup EXIT
# 本地 Ctrl-C / CI 超时：走 exit，让 EXIT trap 负责清理
trap 'exit 130' INT
trap 'exit 143' TERM

fail() {
    echo "FAIL: $*" >&2
    echo "--- gui.log ---" >&2
    tail -n 40 "$TMP/gui.log" 2>/dev/null >&2 || true
    exit 1
}

# 本机缺依赖可以跳过；CI 里设 WEIQI_SMOKE_REQUIRE=1，跳过等于失败
skip() {
    if [ "${WEIQI_SMOKE_REQUIRE:-0}" = 1 ]; then
        echo "FAIL: $*（CI 要求必须跑）" >&2
        exit 2
    fi
    echo "SKIP: $*"
    exit 0
}

# 僵尸进程 kill -0 也返回 0，必须看 /proc 的状态位
alive() {
    local pid=$1 state
    kill -0 "$pid" 2>/dev/null || return 1
    state=$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null) || return 1
    [ "$state" != "Z" ]
}

mkdir -p "$TMP/data/weiqi-study/games"
printf '(;GM[1]FF[4]CA[UTF-8]SZ[19]KM[7.5]RU[Chinese];B[cc];W[gg])\n' \
    > "$TMP/data/weiqi-study/games/game-101.sgf"
printf '(;GM[1]FF[4]CA[UTF-8]SZ[19]KM[7.5]RU[Chinese];B[cc];W[gg];B[dd];W[ee];B[ff];W[hh])\n' \
    > "$TMP/data/weiqi-study/games/game-102.sgf"

if [ -z "${DISPLAY:-}" ]; then
    if ! command -v Xvfb >/dev/null; then
        skip "没有 DISPLAY 也没有 Xvfb"
    fi
    # 挑一个空闲显示号，避免连到残留的 X server
    for n in $(seq 99 110); do
        if [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then
            export DISPLAY=":$n"
            break
        fi
    done
    if [ -z "${DISPLAY:-}" ]; then
        skip "找不到空闲的 X display（:99-:110）"
    fi
    Xvfb "$DISPLAY" -nolisten tcp -screen 0 1280x900x24 >"$TMP/xvfb.log" 2>&1 &
    XVFB_PID=$!
    XVFB_OK=0
    for _ in $(seq 50); do
        if ! kill -0 "$XVFB_PID" 2>/dev/null; then break; fi
        if [ -S "/tmp/.X11-unix/X${DISPLAY#:}" ]; then XVFB_OK=1; break; fi
        sleep 0.1
    done
    if [ "$XVFB_OK" != 1 ]; then
        echo "Xvfb 启动失败：" >&2
        cat "$TMP/xvfb.log" >&2 || true
        exit 2
    fi
fi

export GDK_BACKEND=x11
# 无头环境禁用输入法，避免 fcitx 抢走 xdotool 的按键
export GTK_IM_MODULE=gtk-im-context-simple
export XMODIFIERS=@im=none
# 强制用进程内 GtkFileChooserDialog，避免依赖 xdg-desktop-portal（CI 没有）
export GTK_USE_PORTAL=0
export XDG_DATA_HOME="$TMP/data"
export WEIQI_LANG=zh

# 私有一个 D-Bus 会话：adw::Application 按 application-id 做单实例，
# 共用会话总线时第二次启动会直接把激活转给已有窗口然后退出。
if ! command -v dbus-daemon >/dev/null; then
    skip "需要 dbus-daemon 隔离 adw 单实例"
fi
BUS_INFO=$(dbus-daemon --session --fork --print-address=1 --print-pid=1)
export DBUS_SESSION_BUS_ADDRESS
DBUS_SESSION_BUS_ADDRESS=$(printf '%s\n' "$BUS_INFO" | sed -n 1p)
DBUS_PID=$(printf '%s\n' "$BUS_INFO" | sed -n 2p)

"$BIN" >"$TMP/gui.log" 2>&1 &
APP_PID=$!

HAVE_XDOTOOL=0
if command -v xdotool >/dev/null; then HAVE_XDOTOOL=1; fi

WID=""
if [ "$HAVE_XDOTOOL" = 1 ]; then
    for _ in $(seq 100); do
        WID=$(xdotool search --name 'Weiqi Study' 2>/dev/null | head -1 || true)
        [ -n "$WID" ] && break
        alive "$APP_PID" || break
        sleep 0.1
    done
fi

sleep "$SECONDS_UP"

if ! alive "$APP_PID"; then
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
    # 键盘导航：下一步/上一步/开局/终局/换边（两次回原边）/退出预览
    KEY_FAIL=0
    for k in n n p g G s s Escape; do
        xdotool key --window "$WID" "$k" >/dev/null 2>&1 || KEY_FAIL=$((KEY_FAIL + 1))
        sleep 0.2
    done
    if [ "$KEY_FAIL" -eq 8 ]; then
        fail "8 次按键注入全部失败（xdotool 发不到窗口）"
    fi
    alive "$APP_PID" || fail "键盘导航后进程退出"

    if [ "$WANT_IMPORT" = 1 ]; then
        if [ "$HAVE_XDOTOOL" != 1 ]; then
            if [ "${WEIQI_SMOKE_REQUIRE:-0}" = 1 ]; then
                fail "CI 要求 --import，但没有 xdotool"
            fi
            echo "未安装 xdotool，--import 跳过" >&2
        else
            EXTERNAL="$TMP/import-me.sgf"
            printf '(;GM[1]FF[4]SZ[9]KM[7.5];B[cc];W[gg])\n' > "$EXTERNAL"
            xdotool windowfocus "$WID" >/dev/null 2>&1 || true
            sleep 0.5
            # 用 XTEST（不带 --window），和无头 WM 的焦点模型更兼容
            xdotool key o >/dev/null 2>&1 || true
            DID=""
            for _ in $(seq 50); do
                DID=$(xdotool search --name '导入' 2>/dev/null | head -1 || true)
                [ -z "$DID" ] && DID=$(xdotool search --name 'Import' 2>/dev/null | head -1 || true)
                [ -n "$DID" ] && break
                sleep 0.1
            done
            if [ -z "$DID" ]; then
                echo "--- windows after o ---" >&2
                for w in $(xdotool search --name . 2>/dev/null || true); do
                    n=$(xdotool getwindowname "$w" 2>/dev/null || true)
                    c=$(xdotool getwindowclassname "$w" 2>/dev/null || true)
                    [ -n "$n" ] && echo "  $w: '$n' class=$c" >&2
                done
                echo "focus: $(xdotool getwindowfocus 2>/dev/null || echo none) (main=$WID)" >&2
                fail "按 o 后没有出现文件对话框"
            fi
            xdotool windowfocus "$DID" >/dev/null 2>&1 || true
            xdotool key ctrl+l >/dev/null 2>&1 || true
            sleep 0.4
            # 先导航到临时目录（GtkFileChooserDialog 对 Ctrl+L 输入完整文件路径
            # 不会直接选中文件），再点第二行 import-me.sgf、点“确定”。
            xdotool type "$TMP/" >/dev/null 2>&1 || true
            sleep 0.4
            xdotool key Return >/dev/null 2>&1 || true
            sleep 0.8
            eval "$(xdotool getwindowgeometry --shell "$DID")"
            xdotool mousemove --window "$DID" 200 130 click 1 >/dev/null 2>&1 || true
            sleep 0.3
            xdotool mousemove --window "$DID" $((WIDTH - 40)) $((HEIGHT - 18)) click 1 \
                >/dev/null 2>&1 || true
            sleep 1
            COUNT=$(ls "$XDG_DATA_HOME/weiqi-study/games"/*.sgf 2>/dev/null | wc -l)
            [ "$COUNT" -ge 3 ] || fail "导入后棋谱库只有 $COUNT 盘（期望 >=3）"
            NEWEST=$(ls -t "$XDG_DATA_HOME/weiqi-study/games"/*.sgf | head -1)
            grep -q 'B\[cc\]' "$NEWEST" || fail "导入后的棋谱内容不对：$NEWEST"
            grep -q 'W\[gg\]' "$NEWEST" || fail "导入后的棋谱内容不对：$NEWEST"
            alive "$APP_PID" || fail "导入后进程退出"
        fi
    fi

    if [ "$WANT_SWITCH" = 1 ]; then
        if [ "$HAVE_XDOTOOL" != 1 ]; then
            echo "未安装 xdotool，--switch 跳过" >&2
        else
            # 尽力而为：Tab 到棋谱下拉框（prev/next 之后的第三个控件）再切换。
            xdotool key --window "$WID" Tab Tab Tab >/dev/null 2>&1 || true
            sleep 0.3
            xdotool key --window "$WID" Down >/dev/null 2>&1 || true
            sleep 0.3
            xdotool key --window "$WID" Down Return >/dev/null 2>&1 || true
            sleep 1
            alive "$APP_PID" || fail "切换棋谱后进程退出"
        fi
    fi

    # 帮助浮层放最后：它会抢走焦点，关掉后再做其它操作不可靠
    if [ "$HAVE_XDOTOOL" = 1 ]; then
        xdotool windowfocus "$WID" >/dev/null 2>&1 || true
        xdotool key question >/dev/null 2>&1 || true
        HID=""
        for _ in $(seq 50); do
            HID=$(xdotool search --name '帮助' 2>/dev/null | head -1 || true)
            [ -z "$HID" ] && HID=$(xdotool search --name 'Help' 2>/dev/null | head -1 || true)
            [ -n "$HID" ] && break
            sleep 0.1
        done
        if [ -n "$HID" ]; then
            xdotool windowfocus "$HID" >/dev/null 2>&1 || true
            xdotool key Escape >/dev/null 2>&1 || true
            sleep 0.5
        else
            echo "WARN: 按 ? 没有找到帮助窗口" >&2
        fi
        alive "$APP_PID" || fail "帮助浮层后进程退出"
    fi

    # 按键可能触发后台线程 panic，检查要放在操作之后
    if grep -qE 'panicked at|thread .* panicked|SIGABRT|Aborted' "$TMP/gui.log"; then
        fail "交互后日志里有 panic"
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
for _ in $(seq 30); do alive "$APP_PID" || break; sleep 0.1; done
if alive "$APP_PID"; then
    fail "收到 SIGTERM 后 3s 仍未退出"
fi
APP_PID=""

echo "PASS: $BIN"
