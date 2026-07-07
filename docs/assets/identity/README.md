# forgecode — Identity Demo Media (L105)

Animated SVG + MP4 showcasing the [Terminal-Forge palette](../../assets/tokens.css) in motion.

## Files

| File | Purpose |
|---|---|
| `demo.svg` | 480×270 animated SVG — terminal chrome + scanline + cursor blink + CRT prompt (looped CSS animation, ~5s) |
| `demo.mp4` | H.264/MP4 rendered from `demo.svg` via playwright + ffmpeg (24fps, 5s loop) |

## Palette (Terminal-Forge — amber CRT + synthwave + mint)

- Outer background `#0e0e10`
- Window frame `#1c1c1f`
- Amber CRT `#f5a623` (primary — phosphor + scanline)
- Synthwave `#d946a8` (secondary — AI spark glow)
- Mint prompt `#6ee7b7` (tertiary — prompt + cursor)

## Animation

- CRT scanline: 2.8s linear vertical sweep
- Cursor blink: 1s steps(2) on/off
- Typed text reveal: 4s steps(20) — `forge dispatch --plan "ship-it"`
- Spark core: 2.5s ease-in-out scale + opacity breathing

## Render command

```sh
python /tmp/svg2mp4.py demo.svg demo.mp4 480 270 24 5
```

## Source of truth

- Tokens: [`../../assets/tokens.css`](../../assets/tokens.css)
- Source icon: [`../../assets/brand/forgecode-icon.svg`](../../assets/brand/forgecode-icon.svg)
- Scorecard: `.claude/audit/.vision/L96-L107.md`