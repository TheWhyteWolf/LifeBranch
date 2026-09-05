# lifelock 🔒🫒

A from-scratch [ext-session-lock-v1](https://wayland.app/protocols/ext-session-lock-v1)
screen locker for [niri](https://yalter.github.io/niri/), matching the
olive rice: an **isometric Game of Life cube** rendered in ShureTechMono Nerd
Font shade blocks (░▒▓█) on true black. Each of the cube's three faces runs its own
Conway's Life (the [lifewall](../lifewall) engine); type your password and the
panels flare, the faces flood, and — on success — the session unlocks.

The visuals are cosmetic. Everything security-relevant is modelled on
**swaylock 1.8.6** (see [`NOTICE`](NOTICE)) and, where swaylock is weak,
hardened further.

## What it looks like

- **True black** backdrop with a dim, drifting "ember" Game of Life field.
- A solid **isometric cube**, ~⅓ screen height, its three faces shaded for a
  3D read, each running Life in olive shade blocks.
- **Keystroke** → a random one of the 27 panels flares lime and fades
  (~0.8 s). **Backspace** → a rust flare. Password length is never inferable.
- **Verifying** → a lime wave sweeps the faces. **Wrong** → every cell dies at
  once under a rust flash, then the faces reseed.
- **Clock** (HH:MM) below the cube; a rust **CAPS** tag above it when caps
  lock is on.

Tunables (`--cube-height`, `--pitch`, `--fps`, `--tick`, `--ember-dim`,
`--no-ember`, `--solid-cells`, palette hex flags, …) — run `lifelock --help`.

## Security model

- **Two-process isolation.** PAM runs in a separate `--auth-child` process
  spawned *before* the Wayland connection; the UI process never calls PAM.
  Password-check requests cross a pipe (length-prefixed request, one-byte
  reply). `pam_unix`'s ~2 s failure delay blocks only the child while the UI
  keeps animating "verifying".
- **PAM** through `/etc/pam.d/lifelock` (`auth include login`); the
  conversation hands out the password exactly once per authenticate and aborts
  on any second prompt (systemd-homed guard). Refuses to run setuid. lifelock
  **refuses to start** if the service file is missing (a lock that could never
  unlock is worse than no lock).
- **Password buffer** is a page-aligned, `mlock`ed, `MADV_DONTDUMP` allocation,
  volatile-zeroed at every clear point (submit, Esc, Ctrl+U/C, Ctrl+Backspace,
  backspace-to-empty, 10 s inactivity, and on exit). Capped at 1 KiB.
- **Hardening** (both processes): `RLIMIT_CORE = 0`, `PR_SET_DUMPABLE = 0`,
  `panic = "abort"`; the auth child additionally `mlockall`s — with
  `MCL_FUTURE` only when `RLIMIT_MEMLOCK` allows (a low limit would make PAM's
  own later allocations fail, and an auth child that can't authenticate is a
  session that never unlocks).
- **Protocol.** One lock surface per output, including monitors hot-plugged
  while locked. On the `locked` event — and only then — the `--ready-fd` byte
  and `-f` daemonize handshake fire, so `lifelock -f && systemctl suspend` and
  swayidle's `before-sleep` are race-free. `finished` → exit 2. On auth
  success, `unlock` then a display round-trip before exit. A crash or SIGTERM
  leaves the session locked (the compositor enforces it). By default **PAM is
  the only unlock path** — SIGUSR1 is ignored, so no same-UID process can raise
  the lock. Pass `--allow-signal-unlock` to opt into the scripted
  SIGUSR1-unlock escape hatch (trades that property for convenience).

If the locker dies while locked, niri keeps the session locked and shows a red
screen. Recover by running a locker again (from a TTY, or a keybind with
`allow-when-locked=true`) — it takes over the orphaned lock and unlocks on
auth.

## Build & install

Handled by the rice `install.sh`, which builds the release binary, symlinks it
to `~/.local/bin/lifelock`, and installs the PAM file. Manually:

```sh
cargo build --release
sudo install -Dm644 pam/lifelock /etc/pam.d/lifelock   # required
ln -sfn "$PWD/target/release/lifelock" ~/.local/bin/lifelock
```

Wire it into swayidle in `niri/config.kdl` in place of swaylock:

```
spawn-at-startup "swayidle" "-w" \
    "timeout" "600" "~/.local/bin/lifelock -f" \
    "timeout" "900" "niri msg action power-off-monitors" \
    "lock" "~/.local/bin/lifelock -f" \
    "before-sleep" "~/.local/bin/lifelock -f"
```

`Mod+Alt+Escape` (the lock bind) and the fuzzel power menu need no change — they emit
`loginctl lock-session`, which swayidle turns into a lock.

## Testing

`cargo test` covers the Life engine, isometric projection round-trips, the
password buffer's zeroize/UTF-8/cap behaviour, and the input state machine.
For the compositor path, run a nested niri (`niri -c test.kdl`, opens as a
window) and launch lifelock against its `WAYLAND_DISPLAY` — a crash only
reddens the nested window, never your real session.

## License

[GPL-3.0-or-later](../LICENSE). Logic ported from the MIT-licensed swaylock
is credited in [`NOTICE`](NOTICE), whose MIT text is retained as that licence
requires — MIT permits the incorporation, and the combined work is GPL-3.
