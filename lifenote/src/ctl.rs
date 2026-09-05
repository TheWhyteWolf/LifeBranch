// SPDX-License-Identifier: GPL-3.0-or-later
// `lifenote ctl …` — the makoctl stand-in. A client mode of the same binary
// (lifelock's auth-child dispatch pattern): one blocking proxy call against
// the running daemon's rs.lifenote.Control interface, print, exit.

const CTL_USAGE: &str = "usage: lifenote ctl CMD\n\
      ctl dnd toggle|on|off   switch do-not-disturb (prints new state)\n\
      ctl dnd get             print `on` or `off`\n\
      ctl dismiss-all         close every visible popup\n\
      ctl history             swallowed/expired notifications, newest first\n\
      ctl unseen              count arrived-unseen since history was viewed\n\
      ctl reload              re-read the config file and restyle live\n";

fn onoff(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

pub fn run(args: &[String]) -> i32 {
    let proxy = || -> Result<zbus::blocking::Proxy<'static>, zbus::Error> {
        let conn = zbus::blocking::Connection::session()?;
        zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "rs.lifenote.Control",
        )
    };
    let fail = |e: zbus::Error| -> i32 {
        eprintln!("lifenote ctl: {e} — is the lifenote daemon running?");
        1
    };

    let cmd: Vec<&str> = args.iter().map(String::as_str).collect();
    match cmd.as_slice() {
        ["dnd", sub @ ("toggle" | "on" | "off" | "get")] => {
            let p = match proxy() {
                Ok(p) => p,
                Err(e) => return fail(e),
            };
            let r: Result<bool, zbus::Error> = match *sub {
                "toggle" => p.call("DndToggle", &()),
                "on" => p.call("DndSet", &(true)),
                "off" => p.call("DndSet", &(false)),
                _ => p.call("DndGet", &()),
            };
            match r {
                Ok(on) => {
                    println!("{}", onoff(on));
                    0
                }
                Err(e) => fail(e),
            }
        }
        ["dismiss-all"] => match proxy() {
            Ok(p) => match p.call::<_, _, ()>("DismissAll", &()) {
                Ok(()) => 0,
                Err(e) => fail(e),
            },
            Err(e) => fail(e),
        },
        ["reload"] => match proxy() {
            Ok(p) => match p.call::<_, _, ()>("Reload", &()) {
                Ok(()) => 0,
                Err(e) => fail(e),
            },
            Err(e) => fail(e),
        },
        ["history"] => match proxy() {
            Ok(p) => match p.call::<_, _, Vec<(String, String, String)>>("History", &()) {
                Ok(entries) => {
                    // One physical line per entry is guaranteed at storage:
                    // remember() runs text::one_line (markup + newlines).
                    for (app, summary, body) in entries {
                        if body.is_empty() {
                            println!("[{app}] {summary}");
                        } else {
                            println!("[{app}] {summary} — {body}");
                        }
                    }
                    0
                }
                Err(e) => fail(e),
            },
            Err(e) => fail(e),
        },
        ["unseen"] => match proxy() {
            Ok(p) => match p.call::<_, _, u32>("UnseenCount", &()) {
                Ok(n) => {
                    println!("{n}");
                    0
                }
                Err(e) => fail(e),
            },
            Err(e) => fail(e),
        },
        _ => {
            eprint!("{CTL_USAGE}");
            2
        }
    }
}
