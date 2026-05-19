# pomo

A pomodoro timer and generic countdown CLI with a `ratatui` TUI, desktop
notifications, audible bell, append-only session log, and a `--json` status
subcommand for waybar.

![demo](assets/demo.gif)

## Install

### From source

Requires Rust 1.85+ and a system with ALSA / PulseAudio / PipeWire for sound.

```bash
git clone https://github.com/riccardo-enr/pomo
cd pomo
cargo install --path .
```

This installs `pomo` to `~/.cargo/bin/`. Make sure that directory is on your `$PATH`.

### Prebuilt binary

Download the latest `pomo-x86_64-unknown-linux-gnu` from the [Releases page](https://github.com/riccardo-enr/pomo/releases) and put it on your `$PATH`.

## Usage

```bash
# Generic timer
pomo 25m
pomo 1h30m

# Single pomodoro intervals
pomo work             # 25m work block
pomo break            # 5m short break
pomo long             # 15m long break

# Full pomodoro cycle: N work blocks separated by short breaks, ending with a long break
pomo cycle            # 4 rounds, default durations
pomo cycle -n 3 --work 30m --short 5m --long 20m

# Explicit timer subcommand
pomo timer 90s

# Suppress the bell
pomo --no-sound 25m
```

### TUI controls

| Key            | Action            |
| -------------- | ----------------- |
| `space`        | pause / resume    |
| `r`            | restart current interval |
| `q` / `esc`    | quit early (no bell, no notification) |

Natural completion plays the bell, fires a desktop notification, and appends a record to the session log. An early quit logs the abort with `completed: false` and skips the bell.

## Config

Optional config at `$XDG_CONFIG_HOME/pomo/config.toml` (defaults to `~/.config/pomo/config.toml`). All keys are optional; missing keys use the built-in defaults.

```toml
# Default interval durations (humantime format: "25m", "1h30m", "45s")
work = "25m"
short_break = "5m"
long_break = "15m"

# Default number of work intervals in a `pomo cycle`
rounds = 4

# Bell on interval completion
sound = true
sound_path = ""        # absolute path to a custom WAV; empty = built-in bell
bell_volume = 1.0      # 0.0 .. 4.0

# notify-rust desktop notification on interval completion
desktop_notification = true
```

Precedence: **CLI flag > config file > built-in default**. Unknown keys cause a startup error so typos surface immediately.

## Session log

Every interval (completed or aborted) appends one JSON line to `$XDG_DATA_HOME/pomo/log.jsonl` (defaults to `~/.local/share/pomo/log.jsonl`).

```json
{"started_at":"2026-05-19T22:00:00Z","ended_at":"2026-05-19T22:25:00Z","kind":"work","label":"Work 1/4","planned_secs":1500,"completed":true}
```

Useful queries:

```bash
# Count completed work intervals today
jq -s 'map(select(.kind=="work" and .completed)) | length' ~/.local/share/pomo/log.jsonl

# Total focused minutes this week
jq -s 'map(select(.kind=="work" and .completed) | .planned_secs) | add / 60' ~/.local/share/pomo/log.jsonl
```

## Waybar integration

While a `pomo` session is running, the TUI writes its state (~1 Hz) to `$XDG_RUNTIME_DIR/pomo/state.json`. `pomo status --json` reads that file and prints waybar-compatible JSON.

Sample output:

```json
{"text":"20:34","tooltip":"Work 1/4 (20:34 / 25:00)","class":"work","percentage":18}
{"text":"01:00","tooltip":"Work (paused, 01:00 / 02:00)","class":"paused","percentage":50}
{"text":"","tooltip":"","class":"idle","percentage":0}
```

`class` is one of `work`, `break`, `timer`, `paused`, `idle`. When no pomo is running (or the state file is older than 5 seconds), `status --json` returns the idle object.

### Module config

Add to your `~/.config/waybar/config`:

```json
"custom/pomo": {
    "exec": "pomo status --json",
    "return-type": "json",
    "interval": 1,
    "format": "{} {icon}",
    "format-icons": {
        "work":   "*",
        "break":  ".",
        "timer":  "~",
        "paused": "||",
        "idle":   ""
    },
    "tooltip": true
}
```

Then drop `"custom/pomo"` into one of the `modules-left` / `modules-center` / `modules-right` arrays in the same config.

### Styling

Add to your `~/.config/waybar/style.css`:

```css
#custom-pomo {
    padding: 0 8px;
}
#custom-pomo.work {
    color: #e06c75;
}
#custom-pomo.break {
    color: #98c379;
}
#custom-pomo.timer {
    color: #56b6c2;
}
#custom-pomo.paused {
    color: #abb2bf;
    font-style: italic;
}
#custom-pomo.idle {
    color: transparent;
}
```

## Recording the demo

The `assets/demo.gif` reference above is intentionally absent for v0.1. To record it:

```bash
# Using vhs (https://github.com/charmbracelet/vhs)
vhs assets/demo.tape -o assets/demo.gif

# Or using asciinema + agg
asciinema rec assets/demo.cast
agg assets/demo.cast assets/demo.gif
```

A 10-15s clip of `pomo 10s` is enough.

## License

MIT
