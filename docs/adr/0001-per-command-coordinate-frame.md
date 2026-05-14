# Per-command coordinate frame

Porthole's positioned-input commands (`click`, `scroll`, `text`, and future `pointer move`) take **window-local** coordinates and convert to screen-global internally; window-placement commands (`place`) take **screen-global** coordinates and write them straight to AX; window-inspection commands that report inner geometry (`content-rect`) return **window-local**.

We picked a per-command frame instead of a uniform `--coord-system` flag because each command's natural mental model already implies a frame: clicking *inside* a window is naturally window-local, and moving *a window in space* is naturally screen-global. A uniform flag would force every caller to restate something the command already implies.

The unit axis (`logical` vs `physical` pixels) is orthogonal to the frame axis and *is* exposed as `--units` on input commands, because both units are equally natural depending on where the caller sourced their numbers — see [CONTEXT.md](../../CONTEXT.md#coordinate-units).

## Consequences

- New commands inherit the rule by category, not by personal preference: anything that targets a point inside a window uses window-local; anything that places a window uses screen-global.
- The convention must be explicit in each command's `--help` and in protocol response schemas, because the asymmetry is not visually obvious from the API surface alone.
- A caller composing `place` + `click` will type two different x/y in two different frames in adjacent commands. The example dialogue in CONTEXT.md exists to make this concrete.
