/// Glyphs the UI draws with. Only the logo differs between sets: the status and/// arrow markers stay plain Unicode everywhere so they never render as tofu.
/// The logo uses a Nerd Font glyph when `nerd_fonts: true`, since the terminal
/// font either has it or the user has opted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbols {
    /// Title of the info panel.
    pub zenith: &'static str,
    pub connected: &'static str,
    pub disconnected: &'static str,
    pub active: &'static str,
    pub up: &'static str,
    pub down: &'static str,
}

impl Symbols {
    /// Glyphs present in essentially every terminal font.
    pub const fn unicode() -> Self {
        Symbols {
            zenith: "zenith",
            connected: "●",
            disconnected: "○",
            active: "◉",
            up: "↑",
            down: "↓",
        }
    }

    /// Same as [`Symbols::unicode`] but with a Nerd Font logo in the header.
    pub const fn nerd() -> Self {
        Symbols {
            zenith: "\u{f04e5} zenith", // nf-md-... (U+F04E5) + name
            ..Symbols::unicode()
        }
    }

    pub fn for_setting(nerd_fonts: bool) -> Self {
        if nerd_fonts {
            Symbols::nerd()
        } else {
            Symbols::unicode()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_logo_differs_between_sets() {
        let u = Symbols::unicode();
        let n = Symbols::nerd();
        assert_ne!(u.zenith, n.zenith, "logo changes");
        assert_eq!(u.connected, n.connected);
        assert_eq!(u.disconnected, n.disconnected);
        assert_eq!(u.active, n.active);
        assert_eq!(u.up, n.up);
        assert_eq!(u.down, n.down);
        assert_eq!(Symbols::for_setting(false), Symbols::unicode());
        assert_eq!(Symbols::for_setting(true), Symbols::nerd());
    }

    #[test]
    fn unicode_glyphs_are_nonempty() {
        let s = Symbols::unicode();
        assert!(!s.connected.is_empty() && !s.up.is_empty());
    }
}
