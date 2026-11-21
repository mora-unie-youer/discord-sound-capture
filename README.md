# Discord Sound Capture

Choose which applications are heard in a Discord screen share on Linux (PipeWire + pipewire-pulse).

Discord's screen share creates one capture stream per playback stream (a PulseAudio source-output named `discord_capture`, targeting the app's sink-input). Muting that capture removes the app from the stream. This program mutes every capture by default and leaves only the allowed apps unmuted. Viewers hear what you allow, regardless of what you hear yourself.

## Rules

- **Checkbox** per application, matched by `application.process.binary`.
- **Regex** on one of `application.name`, `application.process.binary`, `media.name`, `node.name`. It matches a substring; anchor with `^…$` for an exact match. A stream is allowed if any rule matches.
- Streams without `application.process.binary` have no checkbox; use a regex.

Rules are saved to `$XDG_CONFIG_HOME/discord-sound-capture/config.toml` (default `~/.config/...`):

```toml
[[rules]]
kind = "binary"
binary = "chrome"
label = "Google Chrome"

[[rules]]
kind = "regex"
field = "application.name"
pattern = "(?i)factorio"
```

A broken config is renamed to `config.toml.broken-<unix time>` and the app starts with no rules.

## Behavior

- While running, all `discord_capture` streams are muted unless allowed. New captures are muted within about a millisecond of appearing, and a 1 s reconcile runs as a safety net.
- On quit, SIGINT, SIGTERM or SIGHUP, all captures are unmuted.
- If the app is killed hard (SIGKILL, crash), existing muted captures stay muted until Discord recreates them.
- Closing the window hides it to the tray. Quit from the tray menu or the window button. Without a tray, closing the window quits.
- Only one instance runs (lock file in `$XDG_RUNTIME_DIR`).

## WirePlumber rule (required)

Without it, WirePlumber restores the saved mute state onto new captures, so they would start muted and stay that way after the app exits. Create `~/.config/wireplumber/wireplumber.conf.d/50-discord-capture.conf`:

```
stream.rules = [
  {
    matches = [ { application.name = "discord_capture" } ]
    actions = { update-props = { state.restore-props = false } }
  }
]
```

Then restart WirePlumber (`systemctl --user restart wireplumber`). The app shows a banner if the rule is not found.

## Build and install

Requires Rust (edition 2024, stable) and `libpulse` headers.

```sh
cargo install --path .        # installs to ~/.cargo/bin
```

## Autostart

`--hidden` starts without opening the window (tray only).

```sh
mkdir -p ~/.config/autostart
sed "s|@BIN@|$HOME/.cargo/bin/discord-sound-capture|" \
  autostart/discord-sound-capture.desktop \
  > ~/.config/autostart/discord-sound-capture.desktop
```

If the tray host (the panel) starts after this app, the tray icon will not appear and closing the window will quit the app. In that case delay the start, e.g. set `Exec=sh -c 'sleep 5; exec /path/to/discord-sound-capture --hidden'`.

## Tests

```sh
cargo test
```
