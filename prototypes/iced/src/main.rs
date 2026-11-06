use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Fill, Length, Size, Task, Theme};
use std::sync::Arc;

const ROW_HEIGHT: f32 = 28.0;
const OVERSCAN: usize = 4;
const ROW_COUNT: usize = 100_000;

pub fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title(|_: &App| "Ferrix Explorer — iced spike".to_string())
        .window_size(Size::new(1000.0, 700.0))
        .theme(|_: &App| Theme::Light)
        .run()
}

struct App {
    rows: Arc<Vec<String>>,
    filtered: Vec<usize>,
    query: String,
    first_visible: usize,
    visible_count: usize,
    selected: Option<usize>,
}

#[derive(Debug, Clone)]
enum Message {
    QueryChanged(String),
    ClearQuery,
    Scrolled(scrollable::Viewport),
    Select(usize),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let rows = Arc::new(
            (0..ROW_COUNT)
                .map(|index| format!("Ferrix sample item {index:06}"))
                .collect::<Vec<_>>(),
        );
        let filtered = (0..rows.len()).collect();
        (
            Self {
                rows,
                filtered,
                query: String::new(),
                first_visible: 0,
                visible_count: 24,
                selected: None,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::QueryChanged(query) => {
                self.query = query;
                let needle = self.query.to_lowercase();
                self.filtered = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        row.to_lowercase().contains(&needle).then_some(index)
                    })
                    .collect();
                self.first_visible = 0;
                self.selected = None;
            }
            Message::ClearQuery => {
                self.query.clear();
                self.filtered = (0..self.rows.len()).collect();
                self.first_visible = 0;
                self.selected = None;
            }
            Message::Scrolled(viewport) => {
                let offset = viewport.absolute_offset().y.max(0.0);
                let height = viewport.bounds().height.max(ROW_HEIGHT);
                self.first_visible = (offset / ROW_HEIGHT).floor() as usize;
                self.visible_count = (height / ROW_HEIGHT).ceil() as usize + 1;
            }
            Message::Select(index) => self.selected = Some(index),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let query = text_input("Filter rows…", &self.query)
            .on_input(Message::QueryChanged)
            .padding(8);
        let clear = button("Clear").on_press(Message::ClearQuery);
        let controls = row![query, clear].spacing(8).align_y(Alignment::Center);

        let total = self.filtered.len();
        let start = self.first_visible.saturating_sub(OVERSCAN).min(total);
        let end = self
            .first_visible
            .saturating_add(self.visible_count)
            .saturating_add(OVERSCAN)
            .min(total);
        let top_height = start as f32 * ROW_HEIGHT;
        let bottom_height = total.saturating_sub(end) as f32 * ROW_HEIGHT;

        let visible_rows = self.filtered[start..end]
            .iter()
            .map(|&source_index| {
                let label = &self.rows[source_index];
                let row = button(text(label).size(14))
                    .width(Fill)
                    .height(Length::Fixed(ROW_HEIGHT))
                    .on_press(Message::Select(source_index));
                container(row).width(Fill).into()
            })
            .collect::<Vec<Element<'_, Message>>>();

        let contents = column![
            space().height(Length::Fixed(top_height)),
            column(visible_rows).width(Fill),
            space().height(Length::Fixed(bottom_height)),
        ]
        .width(Fill);

        let list = scrollable(contents)
            .on_scroll(Message::Scrolled)
            .height(Fill);

        let status = text(format!(
            "Showing {} of {} rows · selected: {} · virtualization window: {} rows",
            end.saturating_sub(start),
            total,
            self.selected
                .map(|index| index.to_string())
                .unwrap_or_else(|| "none".to_string()),
            end.saturating_sub(start),
        ))
        .size(12);

        column![
            text("iced UI spike — virtualized 100,000-row dataset").size(20),
            controls,
            list,
            status,
        ]
        .spacing(10)
        .padding(12)
        .into()
    }
}
