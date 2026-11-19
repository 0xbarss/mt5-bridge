# MT5 bridge autostart

Attach `mt5_bridge` to a chart automatically, without opening the Navigator or
dragging the Expert Advisor onto a chart by hand.

## What "autostart" means here

MetaTrader 5 has **no API to attach an EA to a chart at runtime**, and an EA
cannot run without a chart. So a truly chart-less bridge is not possible with
the current `mt5_bridge.mq5` (it relies on `OnTick`, `OnTradeTransaction`,
`OnBookEvent`, and `OnTimer`; MQL5 *services* receive none of those events).

What you can remove is the **manual configuration step**. The terminal can log
in, open a clean chart, attach the EA, and load its inputs from a file on every
start. That is what this directory does.

## How it works

| File | Purpose |
| :--- | :--- |
| `terminal.ini.example` | `[Common]` auto-login, `[Experts]` trading/DLL permissions, `[StartUp]` chart + EA + preset. |
| `mt5_bridge.set.example` | EA inputs, loaded via `ExpertParameters`. |
| `render_config.sh` | Generates both files from `MT5_*` environment variables. |
| `launch_mt5.sh` / `launch_mt5.bat` | Starts the terminal with `/config:` pointing at the generated INI. |

The key MT5 feature is the `/config:<file>` command-line switch and its
`[StartUp]` section:

```ini
[StartUp]
Expert=mt5_bridge
ExpertParameters=mt5_bridge.set
Symbol=EURUSD
Period=M1
```

`Expert` is relative to `<Terminal_Data_Folder>\MQL5\Experts`, and
`ExpertParameters` is relative to `<Terminal_Data_Folder>\MQL5\Presets`.

## Setup

1. Install the EA and DLL as usual (see the main README):
   - `<data>\MQL5\Experts\mt5_bridge.ex5`
   - `<data>\MQL5\Libraries\mt5_bridge.dll`

2. Fill in `MT5_PIPE_SECRET` (and optionally `MT5_LOGIN`, `MT5_PASSWORD`,
   `MT5_SERVER`, `MT5_SYMBOL`) in the repository-root `.env`. The autostart
   scripts read that same `.env` automatically, so the pipe secret stays in sync
   with the Rust engine.

   ```bash
   # one effective .env — set values here
   $EDITOR .env
   ```

   Omit `MT5_LOGIN`/`MT5_PASSWORD`/`MT5_SERVER` if the account is already saved
   in the terminal (the GUI account database is reused).

3. Generate the config (no exports needed):

   ```bash
   ./mql5/autostart/render_config.sh /secure/config/dir
   ```

4. Copy the preset into the terminal data folder:

   ```bash
   cp /secure/config/dir/mt5_bridge.set \
      "$WINEPREFIX/drive_c/Program Files/MetaTrader 5/MQL5/Presets/mt5_bridge.set"
   ```

5. Start the terminal with the generated INI:

   ```bash
   MT5_CONFIG=/secure/config/dir/terminal.ini ./mql5/autostart/launch_mt5.sh
   ```

   On Windows:

   ```bat
   set MT5_CONFIG=C:\secure\config\dir\terminal.ini
   mql5\autostart\launch_mt5.bat
   ```

   The chart opens and the EA starts by itself. `terminal.ini` is read-only
   while the terminal runs, so changes made in the GUI are not persisted there.

## Refreshing an existing instance

Starting the terminal while an older `mt5_bridge` instance is still running (for
example a chart saved in the profile, or a terminal that was not shut down
cleanly) used to leave the new instance unable to open the pipe. The EA now
refreshes the old instance instead of silently failing:

1. `render_config.sh` writes a monotonic `InpConfigGeneration` into the preset
   (the current Unix time by default).
2. On init, the EA compares its generation with the generation recorded by the
   instance that currently owns the pipe (an MQL5 global variable, shared by all
   EAs in the terminal).
3. If the new generation is higher, it signals a named Win32 event. The old
   instance releases the pipe, unloads itself, and the new instance binds the
   pipe with the freshly rendered inputs.

If the new generation is **not** higher (for example a saved profile chart whose
inputs are older than the running instance), the EA logs that it will not take
over and exits, so two bridges never race for the same pipe.

Set `MT5_CONFIG_GENERATION` to an explicit value to make deployments
reproducible, and set `InpConfigGeneration=0` to disable takeover entirely.
`InpTakeoverTimeoutMs` (default `5000`) bounds how long the new instance waits
for the old one to release the pipe.

> The generation is scoped to the pipe name, so several terminals or accounts
> using different `InpPipeName` values do not interfere with each other.

The autostart preset also sets `InpCloseStaleCharts=true`. Once the winning
instance owns the pipe, it waits three seconds for any takeover to settle and
then closes every other chart. Repeated restarts therefore leave exactly one
chart instead of accumulating duplicates. Because this closes **all** other
charts in the terminal, keep the autostart bridge in a dedicated profile (or a
dedicated terminal) and leave the input `false` for interactive terminals where
you keep analysis charts open.

## Auto-start on boot

Linux/Wine systemd unit:

```ini
[Unit]
Description=MetaTrader 5 bridge
After=network-online.target

[Service]
User=trader
Environment=WINEPREFIX=/home/trader/.wine
Environment=WINEDEBUG=-all
Environment=MT5_CONFIG=/home/trader/.config/mt5/terminal.ini
ExecStart=/opt/mt5-bridge/mql5/autostart/launch_mt5.sh
Restart=always

[Install]
WantedBy=multi-user.target
```

Windows: create a Task Scheduler task at logon that runs `launch_mt5.bat`.

## Caveats

- **Credentials are plain text** in `terminal.ini` and `mt5_bridge.set`. The
  render script creates them with mode `600`; keep them outside the repo and
  readable only by the service user.
- **The chart still exists.** It is a clean, dedicated chart (default
  `EURUSD, M1`) with no indicators. For the lowest-latency `OnTick` path on a
  specific instrument, set `MT5_SYMBOL` to that instrument.
- **`AllowDllImport=1` is required** or `CreateNamedPipe` fails in `OnInit`.
- `InpPipeSDDL` is not written to the generated preset, so the EA's secure
  default (owner/SYSTEM/admins) applies. Add the line manually only if you need
  a custom descriptor.
- **Live accounts** need `InpLiveAckAcknowledged=true` (set `MT5_LIVE_ACK=1`
  before rendering), matching `MT5_LIVE_ACK` on the Rust side.
- If you change the pipe secret, re-render and restart the terminal; the EA
  only reads inputs at init.
