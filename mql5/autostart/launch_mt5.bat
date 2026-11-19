@echo off
REM ============================================================================
REM Start MetaTrader 5 with the autostart mt5_bridge config.
REM The terminal opens the chart and attaches mt5_bridge automatically.
REM
REM Override paths with MT5_TERMINAL / MT5_CONFIG before running.
REM ============================================================================
setlocal

if not defined MT5_TERMINAL set "MT5_TERMINAL=%ProgramFiles%\MetaTrader 5\terminal64.exe"
if not defined MT5_CONFIG set "MT5_CONFIG=%~dp0terminal.ini"

if not exist "%MT5_TERMINAL%" (
    echo error: terminal64.exe not found at "%MT5_TERMINAL%"
    exit /b 1
)
if not exist "%MT5_CONFIG%" (
    echo error: config not found at "%MT5_CONFIG%"
    echo        run render_config.sh first, or set MT5_CONFIG
    exit /b 1
)

start "" "%MT5_TERMINAL%" /config:"%MT5_CONFIG%"
