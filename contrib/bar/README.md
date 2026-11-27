# Status-bar modules

`podctl bar` prints the current state once and then one line per change,
and it keeps running when `podctld` restarts. Every file here is a
ready-made module; copy the snippet into your own config.

| Bar | File | Format |
| --- | --- | --- |
| waybar | `waybar.jsonc`, `waybar.css` | `--format waybar` (default) |
| i3blocks | `i3blocks.conf` | `--format i3blocks` |
| polybar | `polybar.ini` | `--format polybar` |
| Quickshell | `quickshell.qml` | default, parsed as JSON |
| AGS (Astal) | `ags.tsx` | default, parsed as JSON |
| eww | `eww.yuck` | default, parsed as JSON |

The waybar object carries `text`, `alt`, `class`, `tooltip` and
`percentage`. `alt` is `connected`, `disconnected`, `unpaired` or
`offline`, which is what the icon in the examples switches on; `class`
adds `charging`, `low` and, while the buds report where they are, one of
`in-ear`, `one-in-ear`, `out-of-ear` or `in-case`. A device without
in-ear detection — the AirPods Max — carries none of those.
