// SPDX-License-Identifier: GPL-3.0-or-later
// Talking to polkit-agent-helper-1, which runs PAM as root on our behalf.
//
// polkit >= 126 starts it per connection on /run/polkit/agent-helper.socket
// (systemd socket activation). The helper checks the connecting process is
// the registered agent, so this process must connect itself. We send the
// user name and the cookie, one line each; it then sends one line per PAM
// message:
//   PAM_PROMPT_ECHO_OFF <msg>   answer with one line (the password)
//   PAM_PROMPT_ECHO_ON  <msg>   answer with one line (shown as typed)
//   PAM_ERROR_MSG <msg> / PAM_TEXT_INFO <msg>   information, no answer
//   SUCCESS / FAILURE           the end
// <msg> is g_strescape()d. Older polkit has a setuid helper instead: same
// conversation over its stdin/stdout, with the user as argv[1] and only the
// cookie written.

use std::io::{BufRead, BufReader, Read, Write};
use zeroize::Zeroizing;

pub const SOCKET: &str = "/run/polkit/agent-helper.socket";
pub const SETUID_HELPER: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";

/// What the conversation needs from the user.
pub enum Ask<'a> {
    /// A hidden answer (the password). `info` is any PAM text since the last
    /// question, e.g. "Your account will expire".
    Secret { prompt: &'a str, info: &'a str },
    /// A visible answer.
    Visible { prompt: &'a str, info: &'a str },
}

#[derive(Debug, PartialEq)]
pub enum End {
    Success,
    /// PAM said no (wrong password, locked account...). `info` is its message.
    Failure(String),
    /// The user dismissed the prompt.
    Cancelled,
}

/// Undo g_strescape: \n \t \r \b \f \v \\ \" and \ooo octal bytes.
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 == b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        out.push(match c {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'b' => 8,
            b'f' => 12,
            b'v' => 11,
            b'0'..=b'7' => {
                let mut v = (c - b'0') as u32;
                for _ in 0..2 {
                    match b.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            v = v * 8 + (d - b'0') as u32;
                            i += 1;
                        }
                        _ => break,
                    }
                }
                v as u8
            }
            other => other, // \\ \" and anything unexpected: the char itself
        });
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Run one conversation. `ask` returns the answer, or None when the user
/// cancels. Answers live in zeroizing buffers and are wiped after sending.
pub fn converse<S: Read + Write>(
    stream: S,
    greeting: &[u8],
    mut ask: impl FnMut(Ask) -> Option<Zeroizing<String>>,
) -> std::io::Result<End> {
    let mut r = BufReader::new(stream);
    r.get_mut().write_all(greeting)?;
    r.get_mut().flush()?;
    let mut info = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            // Helper gone without a verdict: treat it as a no.
            return Ok(End::Failure(info));
        }
        let l = line.trim_end_matches('\n');
        let (kind, rest) = l.split_once(' ').unwrap_or((l, ""));
        let msg = unescape(rest);
        let answer = match kind {
            "SUCCESS" => return Ok(End::Success),
            "FAILURE" => return Ok(End::Failure(info)),
            "PAM_ERROR_MSG" | "PAM_TEXT_INFO" => {
                if !info.is_empty() {
                    info.push_str(" · ");
                }
                info.push_str(msg.trim());
                continue;
            }
            "PAM_PROMPT_ECHO_OFF" => ask(Ask::Secret { prompt: &msg, info: &info }),
            "PAM_PROMPT_ECHO_ON" => ask(Ask::Visible { prompt: &msg, info: &info }),
            _ => continue, // a line from a newer helper we don't know: ignore
        };
        info.clear();
        let Some(mut answer) = answer else { return Ok(End::Cancelled) };
        // One line per answer: a newline inside would desync the protocol.
        answer.retain(|c| c != '\n' && c != '\r');
        let mut out = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
        out.extend_from_slice(answer.as_bytes());
        out.push(b'\n');
        r.get_mut().write_all(&out)?;
        r.get_mut().flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A fake helper: canned output, and everything we write captured.
    struct Fake {
        out: Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }
    impl Read for Fake {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.out.read(b)
        }
    }
    impl Write for Fake {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.sent.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn fake(script: &str) -> Fake {
        Fake { out: Cursor::new(script.as_bytes().to_vec()), sent: Vec::new() }
    }

    #[test]
    fn password_then_success() {
        let mut f = fake("PAM_TEXT_INFO Hello\\tthere\nPAM_PROMPT_ECHO_OFF Password: \nSUCCESS\n");
        let mut seen = Vec::new();
        let end = converse(&mut f, b"voyd\ncookie-1\n", |a| {
            if let Ask::Secret { prompt, info } = a {
                seen.push((prompt.to_string(), info.to_string()));
            }
            Some(Zeroizing::new("hunter2".to_string()))
        })
        .unwrap();
        assert_eq!(end, End::Success);
        assert_eq!(seen, vec![("Password: ".to_string(), "Hello\tthere".to_string())]);
        assert_eq!(f.sent, b"voyd\ncookie-1\nhunter2\n");
    }

    #[test]
    fn failure_carries_pam_message_and_cancel_sends_nothing() {
        let mut f = fake("PAM_PROMPT_ECHO_OFF Password:\nPAM_ERROR_MSG Account locked\nFAILURE\n");
        let end = converse(&mut f, b"u\nc\n", |_| Some(Zeroizing::new("x".into()))).unwrap();
        assert_eq!(end, End::Failure("Account locked".into()));
        let mut f = fake("PAM_PROMPT_ECHO_OFF Password:\nSUCCESS\n");
        assert_eq!(converse(&mut f, b"u\nc\n", |_| None).unwrap(), End::Cancelled);
        assert_eq!(f.sent, b"u\nc\n", "no answer written on cancel");
    }

    #[test]
    fn newlines_in_an_answer_cannot_inject_lines() {
        let mut f = fake("PAM_PROMPT_ECHO_OFF Password:\nSUCCESS\n");
        converse(&mut f, b"", |_| Some(Zeroizing::new("a\nSUCCESS".into()))).unwrap();
        assert_eq!(f.sent, b"aSUCCESS\n");
    }

    #[test]
    fn eof_is_failure_and_unknown_lines_are_skipped() {
        let mut f = fake("SOMETHING_NEW x\n");
        assert_eq!(converse(&mut f, b"", |_| None).unwrap(), End::Failure(String::new()));
    }

    #[test]
    fn unescapes_like_g_strescape() {
        assert_eq!(unescape(r#"a\nb\t\"q\"\\"#), "a\nb\t\"q\"\\");
        assert_eq!(unescape(r"caf\303\251"), "café");
        assert_eq!(unescape(r"trailing\"), "trailing\\");
    }
}
