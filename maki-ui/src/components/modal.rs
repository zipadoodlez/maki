use crate::theme;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Clear};

pub use maki_lua::Dimension;

/// The border, in cells, on each axis.
pub const BORDER: u16 = 2;

/// The width that keeps `min_cols` visible: never below the preferred width,
/// never above the screen.
pub fn fit_width(preferred: Dimension, min_cols: u16, screen: u16) -> u16 {
    preferred.resolve(screen).max(min_cols).min(screen)
}

pub struct Modal<'a> {
    pub title: &'a str,
    pub width: Dimension,
    pub height: Dimension,
}

impl Modal<'_> {
    /// The popup width: as wide as the content needs, never narrower than the
    /// preferred width, never wider than the screen.
    pub fn width(&self, screen: Rect, content_cols: u16) -> u16 {
        fit_width(self.width, content_cols + BORDER, screen.width)
    }

    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        content_cols: u16,
        content_height: u16,
    ) -> (Rect, Rect) {
        let total_h = (content_height + BORDER)
            .min(self.height.resolve(area.height))
            .max(BORDER + 1);

        let [popup] = Layout::vertical([Constraint::Length(total_h)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Length(self.width(area, content_cols))])
            .flex(Flex::Center)
            .areas(popup);

        frame.render_widget(Clear, popup);

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(theme::current().panel_border)
            .title(self.title)
            .title_style(theme::current().panel_title)
            .style(Style::new().bg(theme::current().background));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        (popup, inner)
    }
}
