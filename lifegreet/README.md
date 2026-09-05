# lifegreet 🚪🫒

A [greetd](https://git.sr.ht/~kennylevinsen/greetd) Wayland greeter for
[niri](https://yalter.github.io/niri/) that makes the login screen look like
the [lifelock](../lifelock) lock screen. It runs as a plain xdg-toplevel
inside the [cage](https://github.com/cage-kiosk/cage) kiosk compositor on vt1
and renders the same isometric Game of Life cube, in ShureTechMono Nerd Font
shade blocks (░▒▓█) on true black.

## The flow

1. Boot lands on an **olive username box** over the ember field, with the
   clock. The username is typed visibly and is required **every** login —
   there is deliberately no `--remember`.
2. **Enter** sends `create_session` to greetd and the **cube grows out of the
   box** (0.8 s ease-out; the faces are already mid-Life — board dimensions
   are pinned to the full-size cube, so no reseed happens while it scales).
3. Password entry is exactly lifelock: **nothing textual appears**. A random
   one of the 27 panels flares lime per keystroke, rust per backspace; Enter
   plays the lime verify wave while greetd runs PAM.
4. Wrong password: rust flash, reseed, and the cube **collapses back into the
   empty username box**. Success: `start_session`, the greeter exits 0, cage
   follows, greetd launches the session.

Keys: `F3` cycles wayland-sessions ("F3: Niri", tiny and dim in the
bottom-left, visible only at the username box), `Esc` on an empty
password backs out to the username box, `Ctrl+U`/`Ctrl+Backspace` clear,
`Ctrl+Alt+Del` reboots. `CAPS` shows above the cube. A partially typed
password self-clears after 10 s (swaylock parity); 45 s idle at an empty
password collapses back to the username box.

## Architecture

The renderer (`sim.rs`, `geometry.rs`, `scene.rs`, `render/`), the password
buffer (`secure_buf.rs`), and the keysym table (`input.rs`) are copies of
lifelock's — fix bugs in both. What differs:

- `app.rs` — xdg window under cage instead of `ext-session-lock-v1` (a
  greeter runs before any session exists, so the lock protocol can't apply).
- `ipc.rs` — a worker thread speaking greetd's length-prefixed JSON protocol
  over `$GREETD_SOCK` (via the `greetd_ipc` crate) instead of lifelock's PAM
  child. greetd owns PAM; the worker thread keeps pam_unix's failure delay
  from freezing the animation.
- `state.rs` — the EnterUser → Growing → EnterPassword → Validating →
  (Failed → Collapsing) → Starting phase machine.
- `sessions.rs` — tiny `.desktop` parser for the `F3` session toggle.
- `geometry.rs` gains `build_grid_map_scaled()` — the grow/collapse frames
  rebuild the cell map at interpolated cube sizes (<1 ms each).

## Security notes

- The password sits in lifelock's mlocked, zeroized `SecureBuf` until submit.
  greetd's protocol ships it as **plaintext JSON over the root-owned socket**
  — that copy (plus serde's transient encode buffer) is inherent to greetd;
  the transient `String` is zeroized right after the socket write.
- `harden.rs` (core dumps off, `PR_SET_DUMPABLE=0`, `mlockall`, refuse
  setuid) runs first thing, same as lifelock. `require_pam_service` is
  dropped — PAM is greetd's job (`/etc/pam.d/greetd`).
- `mlockall(MCL_FUTURE)` only arms when `RLIMIT_MEMLOCK` can hold the whole
  greeter (~112 MiB locked at 1080p — frame buffers count). Under a low limit
  it would be a trap: the call succeeds while the process is small, then every
  later mmap fails `EAGAIN` — under systemd's default 8 MiB that killed the
  greeter at boot before it drew a frame. The installer's service drop-in
  (`greetd/greetd.service.d/lifegreet.conf`) sets `LimitMEMLOCK=infinity` so
  the full lock engages; without it, lifegreet locks only the already-mapped
  pages and relies on `SecureBuf`'s own mlock for the password.
- Panel flares are positioned by the scene RNG only — nothing about the
  password (not even its length, beyond keystroke timing) reaches the screen.
- Debug builds only: `--render-ppm` offscreen renders and the
  `LIFEGREET_TEST_NO_IPC` stub authenticator. Neither exists in release
  (`#[cfg(debug_assertions)]`, same discipline as lifelock).

## Install / rollback

`bash ~/niri/greeter-install.sh` builds the release binary, installs it to
`/usr/local/bin/lifegreet` (it runs as the `greeter` user), installs
`greetd/config.toml` (`cage -s -d -- /usr/local/bin/lifegreet`) plus the
service drop-in (`LimitMEMLOCK=infinity` for the mlockall'd greeter,
`StartLimitIntervalSec=0` so a login screen never gives up), and enables
greetd. **Cut over by rebooting** — never `systemctl restart greetd` from
inside the session it started.

If it breaks: `Ctrl+Alt+F3` to a TTY, then
`sudo install -Dm644 ~/niri/greetd/config-tuigreet.toml /etc/greetd/config.toml
&& sudo systemctl restart greetd` (tuigreet fallback), or disable greetd
entirely and re-enable the previous display manager.

## Development

```sh
cargo test                                # sim/geometry/scene/state/ipc tests
cargo run -- --render-ppm /tmp/g.ppm --phase grow --grow-frac 0.5
LIFEGREET_TEST_NO_IPC=1 cargo run         # in niri: stub auth, "ok" succeeds
python tests/mock-greetd.py /tmp/mock.sock &                    # real protocol
GREETD_SOCK=/tmp/mock.sock cargo run                            # in niri
GREETD_SOCK=/tmp/mock.sock cage -s -d -- ./target/debug/lifegreet  # nested cage
```

Flags mirror lifelock's visual tunables (`--help`), plus `--sessions DIR`,
`--cmd "..."` (fallback session command), and `--user-px` (box text size).
