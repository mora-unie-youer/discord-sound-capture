# Discord Sound Capture

Choose which applications are heard in a Discord screen share on Linux (PipeWire + pipewire-pulse).

Discord's screen share creates one capture stream per playback stream (a PulseAudio source-output named `discord_capture`, targeting the app's sink-input). Muting that capture removes the app from the stream. This program mutes every capture by default and leaves only the allowed apps unmuted. Viewers hear what you allow, regardless of what you hear yourself.

## Rules

- **Checkbox** per application, matched by `application.process.binary`. Streams without a binary show a disabled checkbox with a minus sign.
- **Regex** on one of `application.name`, `application.process.binary`, `media.name`, `node.name`. It matches a substring; anchor with `^…$` for an exact match. A stream is allowed if any enabled rule matches.
- Each regex rule has an enable checkbox. A disabled rule stays in the config but allows nothing. Its status shows `disabled · matches: N`.
- Each application row has two quick buttons, **regex: process** and **regex: name**. They add an exact rule `^<escaped value>$` on `application.process.binary` or `application.name`. A button is inactive if that value is missing. If the rule already exists, it is just enabled again.
- Regex rules are sorted by field (`application.name`, `application.process.binary`, `media.name`, `node.name`), then by pattern. Sorting happens at startup and after a quick button; edits in the list do not reorder it while you type.

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
enabled = true   # optional, defaults to true
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

## Application launcher

```sh
mkdir -p ~/.local/share/applications
sed "s|@BIN@|$HOME/.cargo/bin/discord-sound-capture|" \
  desktop/discord-sound-capture.desktop \
  > ~/.local/share/applications/discord-sound-capture.desktop
```

The app then appears in application menus and launchers and opens its window.

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
