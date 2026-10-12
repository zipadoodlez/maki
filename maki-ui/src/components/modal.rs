use crate::theme;

use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Clear};

pub use maki_lua::Dimension;

pub const CHROME_LINES: u16 = 2;

pub struct Modal<'a> {
    pub title: &'a str,
    pub width: Dimension,
    pub height: Dimension,
}

impl Modal<'_> {
    pub fn render(&self, frame: &mut Frame, area: Rect, content_height: u16) -> (Rect, Rect) {
        let total_h = (content_height + CHROME_LINES)
            .min(self.height.resolve(area.height))
            .max(CHROME_LINES + 1);

        let [popup] = Layout::vertical([Constraint::Length(total_h)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::horizontal([Constraint::Length(self.width.resolve(area.width))])
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
