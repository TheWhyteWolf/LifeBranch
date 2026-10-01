// SPDX-License-Identifier: GPL-3.0-or-later
// lifecursor: writes LifeBranch's cursor themes. Every cursor is drawn from
// shapes (cursors.rs) at each size, so there are no image files to ship.

mod cursors;
mod render;
mod sdf;
mod xcursor;

use std::path::{Path, PathBuf};

const SIZES: &[u32] = &[16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 80, 96, 128];

const USAGE: &str = "usage: lifecursor [--dir DIR] [--sheet FILE.ppm]

Writes the LifeBranch-dark and LifeBranch-light cursor themes into DIR
(default ~/.local/share/icons). --sheet also writes a contact sheet of every
cursor at 48 px on grey, for checking the drawing.";

fn write_theme(root: &Path, name: &str, comment: &str, st: render::Style) -> std::io::Result<()> {
    let dir = root.join(name);
    let cdir = dir.join("cursors");
    std::fs::create_dir_all(&cdir)?;
    std::fs::write(dir.join("index.theme"), format!("[Icon Theme]\nName={name}\nComment={comment}\n"))?;
    for c in cursors::all() {
        let mut images = Vec::new();
        for &size in SIZES {
            let k = size as f32 / 32.0;
            let hot = |v: f32| ((v * k).floor() as u32).min(size - 1);
            for layers in &c.frames {
                images.push(xcursor::Image {
                    size,
                    xhot: hot(c.hot.0),
                    yhot: hot(c.hot.1),
                    delay: c.delay,
                    pixels: render::paint(layers, size, st),
                });
            }
        }
        let tmp = cdir.join(format!(".{}.tmp", c.name));
        std::fs::write(&tmp, xcursor::encode(&images))?;
        std::fs::rename(&tmp, cdir.join(c.name))?;
        for a in c.aliases {
            let link = cdir.join(a);
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(c.name, &link)?;
        }
    }
    Ok(())
}

fn write_sheet(path: &str) -> std::io::Result<()> {
    let (cell, cols) = (64u32, 8u32);
    let cs = cursors::all();
    let styles = [render::DARK, render::LIGHT];
    let rows = (cs.len() as u32).div_ceil(cols) * styles.len() as u32;
    let (w, h) = (cell * cols, cell * rows);
    // Left half of each cell light grey, right half dark: both variants
    // should read on both.
    let mut px: Vec<[f32; 3]> =
        (0..w * h).map(|i| if (i % w) % cell < cell / 2 { [0.85; 3] } else { [0.2; 3] }).collect();
    for (si, st) in styles.iter().enumerate() {
        for (i, c) in cs.iter().enumerate() {
            let i = i as u32;
            let (cx, cy) = ((i % cols) * cell + 8, (i / cols + si as u32 * (rows / 2)) * cell + 8);
            let img = render::paint(&c.frames[0], 48, *st);
            for y in 0..48 {
                for x in 0..48 {
                    let p = img[(y * 48 + x) as usize];
                    let a = (p >> 24) as f32 / 255.0;
                    let d = &mut px[((cy + y) * w + cx + x) as usize];
                    for (j, sh) in [16, 8, 0].iter().enumerate() {
                        d[j] = (p >> sh & 255) as f32 / 255.0 + d[j] * (1.0 - a);
                    }
                }
            }
        }
    }
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.extend(px.iter().flat_map(|c| c.map(|v| (v * 255.0).round() as u8)));
    std::fs::write(path, out)
}

fn main() {
    let mut dir: Option<PathBuf> = None;
    let mut sheet = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--sheet" => sheet = args.next(),
            "-h" | "--help" => return println!("{USAGE}"),
            _ => {
                eprintln!("lifecursor: unknown argument {a:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let dir = dir.unwrap_or_else(|| {
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
        data.join("icons")
    });
    let r = write_theme(&dir, "LifeBranch-dark", "Dark cursors with a white ring: visible on any background", render::DARK)
        .and_then(|_| write_theme(&dir, "LifeBranch-light", "Light cursors with a black ring: visible on any background", render::LIGHT))
        .and_then(|_| sheet.as_deref().map_or(Ok(()), write_sheet));
    match r {
        Ok(()) => println!("lifecursor: wrote LifeBranch-dark and LifeBranch-light to {}", dir.display()),
        Err(e) => {
            eprintln!("lifecursor: {e}");
            std::process::exit(1);
        }
    }
}
