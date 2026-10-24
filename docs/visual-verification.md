# Desktop visual verification in WSL

WSLg supplies an X11 server even when the terminal has no `DISPLAY` variable.
Check that `/mnt/wslg/.X11-unix/X0` exists. The capture script connects to `:0`
automatically in that case. It needs Python 3, `libX11`, and FFmpeg with `x11grab`;
clicking and scrolling also need `libXtst`. No Python packages are required.

Build and launch the native app with X11 and a separate settings directory so the
verification session has its own layout and filters:

```sh
cargo build --release -p ponder-app
mkdir -p target/visual-check/data
DISPLAY=:0 WAYLAND_DISPLAY= XDG_DATA_HOME="$PWD/target/visual-check/data" \
  target/release/ponder-app > target/visual-check/app.log 2>&1 &
app_pid=$!
```

Wait for game data to load, click **Run ladder**, and wait for results. The app uses
the project's existing data, ladder, price, and plan caches. Keep trade pricing
disabled for a local UI check.

Capture only that process's window, resize it, or send a click or scroll before
capturing. Coordinates are pixels relative to the app window; positive scroll
counts move right/down and negative counts move left/up. Input brings the app
window to the foreground and moves the pointer.

```sh
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/wide.png --size 1500 900
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/narrow.png --size 1000 650
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/narrow-right.png --scroll 400 300 12 0
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/narrow-bottom.png --scroll 400 300 0 12
uv run scripts/capture-window.py --pid "$app_pid" \
  --output target/visual-check/selected.png --click 400 150
```

Use `python3` instead of `uv run` if uv is unavailable. Inspect the PNGs with the
agent's image viewer. For table scrolling, compare the leftmost and rightmost
columns, header/body alignment, the last visible row, selection, and sorting.
Screenshots and settings stay under the ignored `target/` directory. Close the
verification app when finished.

Without WSLg, run the same workflow under an Xvfb server with GLX/Mesa support.
Set `DISPLAY` to that server and keep it running for both the app and capture
commands; a running X server is required.
