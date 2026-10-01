# lifeauth

The polkit authentication agent for LifeBranch. When something needs admin
rights (mounting a disk, `pkexec`, lifeconf writing `/etc`), polkitd asks the
session's agent. This one asks you through lifemenu's password box and hands
the answer to polkit's own helper, which runs PAM as root.

It replaces polkit-kde-agent: the same job without Qt, with the prompt in the
desktop's own look. niri starts it at login, and falls back to
polkit-kde-agent (still installed) when lifeauth isn't built. Logs:
`journalctl -t lifeauth`.

## How it works

1. At start it registers on the system bus as the agent for your logind
   session (`$XDG_SESSION_ID`, else the session logind calls your display).
   Only one agent can serve a session, so a running polkit-kde-agent makes it
   exit with a message saying so.
2. polkitd calls `BeginAuthentication` with the reason and the identities
   allowed to authenticate. lifeauth picks you if you're one of them, else the
   first offered (root, or the first admin), and says so in the prompt.
3. It connects to `/run/polkit/agent-helper.socket` (polkit 126+ starts the
   helper per connection; older polkit's setuid helper is spawned instead)
   and relays the PAM conversation. Each question becomes a
   `lifemenu --password --prompt-only` box with polkit's message under it.
4. A wrong password re-asks, up to three times, showing PAM's own message
   when it gives one. Esc cancels. If the requesting program goes away,
   polkitd's `CancelAuthentication` closes the open prompt.

Passwords are kept in zeroizing buffers in lifeauth. They cross from the
prompt over a pipe, never a command line.

## Testing

`cargo test` covers the helper protocol against a fake helper (prompts, PAM
messages, cancel, injection of newlines, escaping), identity choice and the
prompt text. A live check, with polkit-kde-agent stopped:

```sh
~/.local/bin/lifeauth &
pkexec true        # the prompt appears; the right password exits 0
```
