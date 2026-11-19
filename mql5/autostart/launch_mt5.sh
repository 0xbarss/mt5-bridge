#!/usr/bin/env bash
# =============================================================================
# Start MetaTrader 5 under Wine with the autostart mt5_bridge config.
#
# The terminal opens the chart and attaches mt5_bridge automatically; no
# Navigator drag-and-drop is required.
#
# Values are read from the single repository-root .env. Explicit environment
# variables and the overrides below take precedence.
#
# Environment overrides:
#   MT5_TERMINAL  Path to terminal64.exe (default: ~/.wine/drive_c/Program Files/MetaTrader 5/terminal64.exe)
#   MT5_CONFIG    Path to terminal.ini   (default: ./terminal.ini next to this script)
#   WINEPREFIX    Wine prefix            (default: ~/.wine)
#   MT5_ENV_FILE  Path to the .env file  (default: ../../.env relative to this script)
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=env.sh
source "$SCRIPT_DIR/env.sh"
load_mt5_env "${MT5_ENV_FILE:-$SCRIPT_DIR/../../.env}"

TERMINAL="${MT5_TERMINAL:-$HOME/.wine/drive_c/Program Files/MetaTrader 5/terminal64.exe}"
CONFIG="${MT5_CONFIG:-$SCRIPT_DIR/terminal.ini}"

if [[ ! -f "$TERMINAL" ]]; then
    echo "error: terminal64.exe not found at: $TERMINAL" >&2
    echo "       set MT5_TERMINAL to its actual path" >&2
    exit 1
fi
if [[ ! -f "$CONFIG" ]]; then
    echo "error: config not found at: $CONFIG" >&2
    echo "       run ./render_config.sh first, or set MT5_CONFIG" >&2
    exit 1
fi

export WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"
export WINEDEBUG="${WINEDEBUG:--all}"

CONFIG_WIN="$(winepath -w "$CONFIG")"

exec wine "$TERMINAL" /config:"$CONFIG_WIN" "$@"
