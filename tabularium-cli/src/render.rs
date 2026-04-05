//! Shared terminal markdown skin (`termimad`) for CLI and chat TUI.

use termimad::MadSkin;

/// Default skin; `NO_COLOR` disables styling (plain structural output).
pub(crate) fn mad_skin() -> MadSkin {
    if std::env::var_os("NO_COLOR").is_some() {
        MadSkin::no_style()
    } else {
        MadSkin::default()
    }
}
