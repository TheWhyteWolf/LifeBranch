# Contributing

This is one person's desktop, published because the pieces are reusable. Bug
reports from hardware I do not have are the most useful thing you can send:
most of this repo is hardware guesswork with the guesses written down, so a
machine that disproves one is worth knowing about.

Open an issue before starting a feature. The answer may be "that is a fork",
which is fine; every choice here is a plain config file so forking is cheap.

## Rules that matter

**Nothing touches `/etc` or the login path without a way back.** The greeter
installer stages its PAM stack under a scratch service name, tests that, and
only promotes it once it answers, because a broken `/etc/pam.d/greetd` means
nobody can log in. Keep that shape, and put the rollback in the script where
someone reading it at 2am will find it.

**Both installers, or neither.** `install.sh` and `macbook/install.sh` link the
same scripts and write the same fenced config regions. A change to one that
skips the other has been this repo's most common bug. Same for
`niri/config.kdl` and `macbook/niri/config.kdl`: the MacBook uses the function
row for brightness and volume, so check for bind collisions.

**Comments say why, not what.** Most of the comments here exist because
something failed once and the fix looks arbitrary without the story.

## Before a PR

```sh
for f in bootstrap.sh install.sh greeter-install.sh macbook/install.sh scripts/*.sh; do bash -n "$f"; done
shellcheck -S warning -x bootstrap.sh install.sh greeter-install.sh macbook/install.sh scripts/*.sh
niri validate --config niri/config.kdl
niri validate --config macbook/niri/config.kdl
for c in lifewall lifelock lifenote lifeconf lifegreet; do cargo build --release --manifest-path $c/Cargo.toml; done
```

shellcheck is gated at `-S warning`; `info` and `style` notes are advisory.

`scripts/pinentry-fuzzel.sh` speaks Assuan on stdin/stdout, so it is testable
with a stub `fuzzel` on `PATH` that echoes a fixed answer. Test the cancel path
(stub exits 1), the empty-passphrase path (exits 0, no output, which is a real
empty passphrase and not a cancel), and `SETREPEAT` (asks twice, emits
`S PIN_REPEATED 1`).

## Licensing

GPL-3.0-or-later. Patches go out under the same terms; there is no CLA. New
files need `SPDX-License-Identifier: GPL-3.0-or-later` (`//` for Rust, after
the shebang for scripts).
