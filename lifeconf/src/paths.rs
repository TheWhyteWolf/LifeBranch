// SPDX-License-Identifier: GPL-3.0-or-later
// Where everything lives. All consumer configs are written to their standard
// XDG locations under $HOME/.config; on an installed rice those paths are
// symlinks back into the repo (install.sh), so writing here transparently
// updates the tracked source too. LIFECONF_HOME overrides the base for tests
// so a smoke run never touches the user's live dotfiles.

pub struct Paths {
    pub home: String,
}

impl Paths {
    pub fn resolve() -> Paths {
        let home = std::env::var("LIFECONF_HOME")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into());
        Paths { home }
    }

    pub fn config(&self, rel: &str) -> String {
        format!("{}/.config/{}", self.home, rel)
    }

    pub fn cache(&self, rel: &str) -> String {
        format!("{}/.cache/{}", self.home, rel)
    }

    /// theme.toml — lifeconf's own canonical state file.
    pub fn theme_toml(&self) -> String {
        self.config("lifeconf/theme.toml")
    }
}
