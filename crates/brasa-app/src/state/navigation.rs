//! The activity rail's state (GitHub #64): which section the sidebar shows and whether the
//! sidebar is collapsed. Plain Rust, unit tested without a window.

/// A section of the activity rail. Each one swaps the sidebar's content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavSection {
    /// The request tree (the original sidebar).
    #[default]
    Collections,
    /// The workspace's git changes (milestone 0.5.0).
    SourceControl,
    /// The workspace's environments.
    Environments,
    /// The load test targets and the load tests open in this session.
    LoadTests,
}

impl NavSection {
    /// Every section, in the order the rail lists them.
    pub const ALL: [NavSection; 4] = [
        NavSection::Collections,
        NavSection::SourceControl,
        NavSection::Environments,
        NavSection::LoadTests,
    ];
}

/// The rail's selection: the active section and whether its sidebar is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavState {
    section: NavSection,
    collapsed: bool,
}

impl NavState {
    /// Handles a click on a rail button, VS Code style: another section switches to it,
    /// the active one collapses the sidebar, and any click while collapsed expands it showing
    /// the clicked section.
    pub fn click(&mut self, section: NavSection) {
        if self.collapsed {
            self.collapsed = false;
            self.section = section;
        } else if self.section == section {
            self.collapsed = true;
        } else {
            self.section = section;
        }
    }

    /// The section the sidebar shows (or would show once expanded).
    pub fn section(&self) -> NavSection {
        self.section
    }

    /// Whether the sidebar is shown.
    pub fn sidebar_visible(&self) -> bool {
        !self.collapsed
    }

    /// Collapses the sidebar outright, whatever section is currently active. Unlike
    /// [`Self::click`], this never expands it: the rail stays where it is, only the sidebar
    /// hides, for the sidebar's own dedicated "collapse" button.
    pub fn collapse(&mut self) {
        self.collapsed = true;
    }

    /// Whether the rail button for `section` is drawn as active: it is the current section and
    /// the sidebar is open.
    pub fn is_active(&self, section: NavSection) -> bool {
        !self.collapsed && self.section == section
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_on_collections_with_the_sidebar_open() {
        let nav = NavState::default();
        assert_eq!(nav.section(), NavSection::Collections);
        assert!(nav.sidebar_visible());
    }

    #[test]
    fn clicking_another_section_switches_to_it() {
        let mut nav = NavState::default();
        nav.click(NavSection::LoadTests);
        assert_eq!(nav.section(), NavSection::LoadTests);
        assert!(nav.sidebar_visible());
        assert!(nav.is_active(NavSection::LoadTests));
        assert!(!nav.is_active(NavSection::Collections));
    }

    #[test]
    fn clicking_the_active_section_collapses_the_sidebar() {
        let mut nav = NavState::default();
        nav.click(NavSection::Collections);
        assert!(!nav.sidebar_visible());
        assert!(!nav.is_active(NavSection::Collections));
    }

    #[test]
    fn clicking_any_section_while_collapsed_expands_it_on_that_section() {
        let mut nav = NavState::default();
        nav.click(NavSection::Collections);
        nav.click(NavSection::Environments);
        assert!(nav.sidebar_visible());
        assert_eq!(nav.section(), NavSection::Environments);
    }

    #[test]
    fn clicking_the_previous_section_while_collapsed_reopens_it() {
        let mut nav = NavState::default();
        nav.click(NavSection::Collections);
        nav.click(NavSection::Collections);
        assert!(nav.sidebar_visible());
        assert_eq!(nav.section(), NavSection::Collections);
    }

    #[test]
    fn collapse_hides_the_sidebar_without_changing_the_section() {
        let mut nav = NavState::default();
        nav.click(NavSection::Environments);
        nav.collapse();
        assert!(!nav.sidebar_visible());
        assert_eq!(nav.section(), NavSection::Environments);
    }
}
