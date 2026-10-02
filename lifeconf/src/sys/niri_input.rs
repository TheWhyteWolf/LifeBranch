// SPDX-License-Identifier: GPL-3.0-or-later
// Shared plumbing for the panels that edit niri's `input` settings. They live
// in fenced `// LIFEBRANCH:BEGIN <name>` regions of local.kdl, the per-machine file config.kdl includes (the installer
// owns those too); we parse the region, change the nodes we own, render it back
// and write it through the same stage -> `niri validate` -> rename path lifeconf
// uses for its theme regions. niri hot-reloads config.kdl, so a successful
// write is live immediately; a rejected one leaves the config untouched.

use super::kdl::{self, Node};
use crate::gen::niri_kdl::{region_body, replace_region_in, write_validated};

const PREFIX: &str = "LIFEBRANCH";
const NOTE: &str = "    // Managed by lifeconf (Settings). Settings it doesn't know are kept.";

pub fn config_path() -> String {
    let home = std::env::var("LIFECONF_HOME")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    format!("{home}/.config/niri/local.kdl")
}

/// Parse the region's top-level nodes. Err names what we can't safely edit.
pub fn read(path: &str, region: &str) -> Result<Vec<Node>, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let body = region_body(&src, PREFIX, region)
        .ok_or_else(|| format!("no LIFEBRANCH:BEGIN {region} region in niri config"))?;
    kdl::parse(body).map_err(|e| format!("{region} region isn't plain enough to edit here ({e}); edit local.kdl by hand"))
}

pub fn write(path: &str, region: &str, nodes: &[Node]) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut inner = String::from(NOTE);
    let body = kdl::render(nodes, 1);
    if !body.is_empty() {
        inner.push('\n');
        inner.push_str(&body);
    }
    let next = replace_region_in(&src, PREFIX, region, &inner)
        .ok_or_else(|| format!("no LIFEBRANCH:BEGIN {region} region in niri config"))?;
    write_validated(path, &next).map_err(|e| e.trim_start_matches("! ").to_string())
}

/// Characters xkb layout/variant/option names are made of. Anything else would
/// be KDL syntax, not a name.
pub fn xkb_name_ok(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | ',' | '+' | '-' | '(' | ')'))
}

#[cfg(test)]
pub fn test_config(tag: &str, region: &str, inner: &str) -> String {
    let dir = std::env::temp_dir().join(format!("lc-input-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config.kdl");
    std::fs::write(
        &p,
        format!("input {{\n    // LIFEBRANCH:BEGIN {region}\n{inner}\n    // LIFEBRANCH:END {region}\n\n    mouse {{}}\n}}\n"),
    )
    .unwrap();
    p.to_string_lossy().into_owned()
}
