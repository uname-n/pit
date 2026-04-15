use crate::db::Db;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use serde_json::{json, Value};
use std::{
    io,
    time::{Duration, Instant},
};

const COL_OPEN: Color = Color::Blue;
const COL_INPROGRESS: Color = Color::Yellow;
const COL_CLOSED: Color = Color::Green;
const DIM: Color = Color::DarkGray;
const MUTED: Color = Color::Gray;

struct IssueCard {
    id: i64,
    title: String,
    priority: Option<String>,
}

struct Column {
    status: &'static str,
    title: &'static str,
    accent: Color,
    issues: Vec<IssueCard>,
    state: ListState,
}

struct App {
    columns: [Column; 3],
    selected_col: usize,
    last_refresh: Instant,
    error: Option<String>,
    total: usize,
    detail: Option<Value>,
    detail_id: Option<i64>,
    detail_scroll: u16,
    column_rects: [Rect; 3],
    detail_rect: Option<Rect>,
}

impl App {
    fn new() -> Self {
        Self {
            columns: [
                Column {
                    status: "open",
                    title: "Open",
                    accent: COL_OPEN,
                    issues: vec![],
                    state: ListState::default(),
                },
                Column {
                    status: "in-progress",
                    title: "In Progress",
                    accent: COL_INPROGRESS,
                    issues: vec![],
                    state: ListState::default(),
                },
                Column {
                    status: "closed",
                    title: "Closed",
                    accent: COL_CLOSED,
                    issues: vec![],
                    state: ListState::default(),
                },
            ],
            selected_col: 0,
            last_refresh: Instant::now() - Duration::from_secs(3600),
            error: None,
            total: 0,
            detail: None,
            detail_id: None,
            detail_scroll: 0,
            column_rects: [Rect::new(0, 0, 0, 0); 3],
            detail_rect: None,
        }
    }

    fn current_issue_id(&self) -> Option<i64> {
        let col = &self.columns[self.selected_col];
        col.state
            .selected()
            .and_then(|i| col.issues.get(i))
            .map(|c| c.id)
    }

    fn close_detail(&mut self) {
        self.detail = None;
        self.detail_id = None;
        self.detail_scroll = 0;
    }

    fn open_detail(&mut self, db: &Db, id: i64) {
        self.detail_id = Some(id);
        self.detail_scroll = 0;
        self.load_detail(db);
    }

    fn toggle_detail(&mut self, db: &Db) {
        if self.detail.is_some() {
            self.close_detail();
            return;
        }
        if let Some(id) = self.current_issue_id() {
            self.open_detail(db, id);
        }
    }

    fn load_detail(&mut self, db: &Db) {
        let Some(id) = self.detail_id else {
            self.detail = None;
            return;
        };
        match db.get_issue(&json!({ "id": id })) {
            Ok(v) => self.detail = Some(v),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn refresh(&mut self, db: &Db) {
        self.error = None;
        self.total = 0;
        for col in &mut self.columns {
            let sort = if col.status == "open" { "created" } else { "updated" };
            let args = json!({
                "status": col.status,
                "limit": 200,
                "sort": sort,
                "order": "desc",
            });
            match db.list_issues(&args) {
                Ok(v) => {
                    let prev = col.state.selected();
                    col.issues = v.get("issues")
                        .and_then(Value::as_array)
                        .map(|arr| arr.iter().map(parse_card).collect())
                        .unwrap_or_default();
                    self.total += col.issues.len();
                    if col.issues.is_empty() {
                        col.state.select(None);
                    } else {
                        let idx = prev.unwrap_or(0).min(col.issues.len() - 1);
                        col.state.select(Some(idx));
                    }
                }
                Err(e) => {
                    self.error = Some(e.to_string());
                }
            }
        }
        if let Some(did) = self.detail_id {
            let mut located: Option<(usize, usize)> = None;
            for (ci, col) in self.columns.iter().enumerate() {
                if let Some(idx) = col.issues.iter().position(|c| c.id == did) {
                    located = Some((ci, idx));
                    break;
                }
            }
            if let Some((ci, idx)) = located {
                self.selected_col = ci;
                self.columns[ci].state.select(Some(idx));
                self.load_detail(db);
            } else {
                self.close_detail();
            }
        }
        self.last_refresh = Instant::now();
    }

    fn move_col(&mut self, delta: i32) {
        let n = self.columns.len() as i32;
        self.selected_col = (((self.selected_col as i32 + delta) % n + n) % n) as usize;
    }

    fn move_item(&mut self, delta: i32) {
        let col = &mut self.columns[self.selected_col];
        if col.issues.is_empty() {
            return;
        }
        let len = col.issues.len() as i32;
        let cur = col.state.selected().unwrap_or(0) as i32;
        let next = (((cur + delta) % len) + len) % len;
        col.state.select(Some(next as usize));
    }
}

fn parse_card(v: &Value) -> IssueCard {
    IssueCard {
        id: v.get("id").and_then(Value::as_i64).unwrap_or(0),
        title: v.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
        priority: v.get("priority").and_then(Value::as_str).map(String::from),
    }
}

pub fn run(db: &Db) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.refresh(db);

    let res = event_loop(&mut terminal, &mut app, db);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    res
}

fn event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut App, db: &Db) -> io::Result<()> {
    let refresh = Duration::from_secs(2);
    loop {
        terminal.draw(|f| render(f, app))?;

        let timeout = refresh
            .checked_sub(app.last_refresh.elapsed())
            .unwrap_or(Duration::from_millis(0));

        if event::poll(timeout)? {
            let ev = event::read()?;
            if let Event::Mouse(m) = ev {
                handle_mouse(app, db, m);
                continue;
            }
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
                {
                    return Ok(());
                }
                if app.detail.is_some() {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Esc | KeyCode::Enter => {
                            app.close_detail();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.detail_scroll = app.detail_scroll.saturating_add(1);
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.detail_scroll = app.detail_scroll.saturating_sub(1);
                        }
                        KeyCode::PageDown | KeyCode::Char('J') => {
                            app.detail_scroll = app.detail_scroll.saturating_add(5);
                        }
                        KeyCode::PageUp | KeyCode::Char('K') => {
                            app.detail_scroll = app.detail_scroll.saturating_sub(5);
                        }
                        KeyCode::Home | KeyCode::Char('g') => {
                            app.detail_scroll = 0;
                        }
                        KeyCode::Char('G') | KeyCode::End => {
                            app.detail_scroll = u16::MAX;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Enter => app.toggle_detail(db),
                        KeyCode::Char('r') => app.refresh(db),
                        KeyCode::Down | KeyCode::Char('j') => app.move_item(1),
                        KeyCode::Up | KeyCode::Char('k') => app.move_item(-1),
                        KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                            app.move_col(-1)
                        }
                        KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                            app.move_col(1)
                        }
                        KeyCode::Home | KeyCode::Char('g') => {
                            let col = &mut app.columns[app.selected_col];
                            if !col.issues.is_empty() {
                                col.state.select(Some(0));
                            }
                        }
                        KeyCode::Char('G') | KeyCode::End => {
                            let col = &mut app.columns[app.selected_col];
                            if !col.issues.is_empty() {
                                col.state.select(Some(col.issues.len() - 1));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        if app.last_refresh.elapsed() >= refresh {
            app.refresh(db);
        }
    }
}

fn render(f: &mut Frame, app: &mut App) {
    app.detail_rect = None;
    let has_detail = app.detail.is_some();
    let body_constraints = if has_detail {
        vec![Constraint::Percentage(55), Constraint::Percentage(45)]
    } else {
        vec![Constraint::Min(5)]
    };

    let mut outer = vec![
        Constraint::Length(1),
        Constraint::Length(1),
    ];
    outer.extend(body_constraints);
    outer.push(Constraint::Length(1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(outer)
        .split(f.area());

    render_header(f, chunks[0], app);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(f.area().width as usize),
            Style::default().fg(DIM),
        ))),
        chunks[1],
    );
    render_columns(f, chunks[2], app);
    if has_detail {
        render_detail(f, chunks[3], app);
        render_footer(f, chunks[4], app);
    } else {
        render_footer(f, chunks[3], app);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let elapsed = app.last_refresh.elapsed().as_secs();
    let refresh_msg = if elapsed < 2 {
        "just now".to_string()
    } else {
        format!("{elapsed}s ago")
    };

    let mut left = vec![
        Span::styled(
            " pit kanban ",
            Style::default()
                .fg(COL_OPEN)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("read-only", Style::default().fg(MUTED)),
        Span::raw("  ·  "),
        Span::styled(
            format!("{} issues", app.total),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  ·  "),
        Span::styled(format!("refreshed {refresh_msg}"), Style::default().fg(DIM)),
    ];
    if let Some(err) = &app.error {
        left.push(Span::raw("  "));
        left.push(Span::styled(
            format!("error: {err}"),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let key = |k: &'static str| {
        Span::styled(
            format!(" {k} "),
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )
    };
    let sep = || Span::styled("  ·  ", Style::default().fg(DIM));
    let lbl = |s: String| Span::styled(s, Style::default());

    let line = if app.detail.is_some() {
        Line::from(vec![
            Span::raw(" "),
            key("↑↓"), Span::raw(" "), lbl("scroll".into()),
            sep(),
            key("PgUp/Dn"), Span::raw(" "), lbl("page".into()),
            sep(),
            key("⏎/esc"), Span::raw(" "), lbl("close".into()),
            sep(),
            key("q"), Span::raw(" "), lbl("quit".into()),
        ])
    } else {
        Line::from(vec![
            Span::raw(" "),
            key("←→"), Span::raw(" "), lbl("column".into()),
            sep(),
            key("↑↓"), Span::raw(" "), lbl("card".into()),
            sep(),
            key("⏎"), Span::raw(" "), lbl("open".into()),
            sep(),
            key("r"), Span::raw(" "), lbl("refresh".into()),
            sep(),
            key("q"), Span::raw(" "), lbl("quit".into()),
        ])
    };
    f.render_widget(Paragraph::new(line), area);
}

fn render_columns(f: &mut Frame, area: Rect, app: &mut App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
            Constraint::Ratio(1, 3),
        ])
        .split(area);

    for i in 0..3 {
        app.column_rects[i] = cols[i];
    }
    let selected = app.selected_col;
    for (i, col) in app.columns.iter_mut().enumerate() {
        render_column(f, cols[i], col, i == selected);
    }
}

fn render_column(f: &mut Frame, area: Rect, col: &mut Column, is_selected: bool) {
    let border_style = if is_selected {
        Style::default().fg(col.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(DIM)
    };

    let title_line = Line::from(vec![
        Span::styled(
            format!(" {} ", col.title),
            Style::default().fg(col.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ", col.issues.len()),
            Style::default().fg(MUTED),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(title_line)
        .padding(Padding::new(0, 1, 0, 0));

    if col.issues.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  no issues",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )))
        .block(block);
        f.render_widget(empty, area);
        return;
    }

    let inner_width = area.width.saturating_sub(5) as usize;
    let items: Vec<ListItem> = col
        .issues
        .iter()
        .map(|c| card_to_item(c, inner_width))
        .collect();
    let mut list = List::new(items)
        .block(block)
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);

    let mut state_for_render;
    let state_ref = if is_selected {
        list = list
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▌ ");
        &mut col.state
    } else {
        list = list.highlight_symbol("  ");
        state_for_render = ListState::default();
        &mut state_for_render
    };
    f.render_stateful_widget(list, area, state_ref);
}

fn priority_mark(p: Option<&str>) -> Span<'static> {
    match p {
        Some("p0") => Span::styled("●", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Some("p1") => Span::styled("●", Style::default().fg(Color::LightRed)),
        Some("p2") => Span::styled("●", Style::default().fg(Color::Yellow)),
        Some("p3") => Span::styled("●", Style::default().fg(Color::Green)),
        _ => Span::styled("·", Style::default().fg(DIM)),
    }
}

fn card_to_item(card: &IssueCard, width: usize) -> ListItem<'_> {
    let mark = priority_mark(card.priority.as_deref());
    let id = format!("#{}", card.id);

    let fixed = 1 + 1 + id.len() + 1;
    let budget = width.saturating_sub(fixed);
    let title = if card.title.chars().count() > budget {
        truncate(&card.title, budget)
    } else {
        card.title.clone()
    };

    let spans = vec![
        mark,
        Span::raw(" "),
        Span::styled(id, Style::default().fg(MUTED)),
        Span::raw(" "),
        Span::styled(title, Style::default()),
    ];
    ListItem::new(Line::from(spans))
}

fn render_detail(f: &mut Frame, area: Rect, app: &mut App) {
    app.detail_rect = Some(area);
    let Some(v) = &app.detail else { return };
    let id = v.get("id").and_then(Value::as_i64).unwrap_or(0);
    let title = v.get("title").and_then(Value::as_str).unwrap_or("");
    let body = v.get("body").and_then(Value::as_str).unwrap_or("");
    let status = v.get("status").and_then(Value::as_str).unwrap_or("");
    let priority = v.get("priority").and_then(Value::as_str);
    let closed_reason = v.get("closed_reason").and_then(Value::as_str);
    let created_at = v.get("created_at").and_then(Value::as_str).unwrap_or("");
    let updated_at = v.get("updated_at").and_then(Value::as_str).unwrap_or("");
    let labels: Vec<&str> = v
        .get("labels")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|l| l.as_str()).collect())
        .unwrap_or_default();
    let comments = v
        .get("comments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let links = v
        .get("links")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let status_accent = match status {
        "open" => COL_OPEN,
        "in-progress" => COL_INPROGRESS,
        "closed" => COL_CLOSED,
        _ => MUTED,
    };

    let title_line = Line::from(vec![
        Span::raw(" "),
        priority_mark(priority),
        Span::raw(" "),
        Span::styled(format!("#{id}"), Style::default().fg(DIM)),
        Span::raw("  "),
        Span::styled(
            title.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(status_accent).add_modifier(Modifier::BOLD))
        .title(title_line)
        .padding(Padding::new(2, 2, 1, 1));

    let mut lines: Vec<Line> = Vec::new();

    let mut meta = vec![
        Span::styled("status ", Style::default().fg(DIM)),
        Span::styled(
            status.to_string(),
            Style::default().fg(status_accent).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(p) = priority {
        meta.push(Span::styled("   priority ", Style::default().fg(DIM)));
        meta.push(Span::styled(p.to_string(), Style::default()));
    }
    if let Some(r) = closed_reason {
        meta.push(Span::styled("   reason ", Style::default().fg(DIM)));
        meta.push(Span::styled(r.to_string(), Style::default()));
    }
    lines.push(Line::from(meta));

    if !labels.is_empty() {
        let mut spans = vec![Span::styled("labels ", Style::default().fg(DIM))];
        for (i, l) in labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" · ", Style::default().fg(DIM)));
            }
            spans.push(Span::styled(
                l.to_string(),
                Style::default().fg(Color::Magenta),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(vec![
        Span::styled("created ", Style::default().fg(DIM)),
        Span::styled(created_at.to_string(), Style::default().fg(MUTED)),
        Span::styled("   updated ", Style::default().fg(DIM)),
        Span::styled(updated_at.to_string(), Style::default().fg(MUTED)),
    ]));

    lines.push(Line::from(""));

    if body.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no description)",
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )));
    } else {
        lines.extend(parse_markdown(body, ""));
    }

    if !links.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("── links ({}) ──", links.len()),
            Style::default().fg(DIM),
        )));
        for l in &links {
            let lt = l.get("link_type").and_then(Value::as_str).unwrap_or("");
            let src = l.get("source_id").and_then(Value::as_i64).unwrap_or(0);
            let tgt = l.get("target_id").and_then(Value::as_i64).unwrap_or(0);
            let (arrow, other) = if src == id {
                ("→", tgt)
            } else {
                ("←", src)
            };
            let accent = match lt {
                "blocks" => Color::Red,
                "duplicates" => Color::Magenta,
                _ => Color::Cyan,
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {lt:<11}"),
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(arrow.to_string(), Style::default().fg(DIM)),
                Span::raw(" "),
                Span::styled(format!("#{other}"), Style::default().fg(MUTED)),
            ]));
        }
    }

    if !comments.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("── comments ({}) ──", comments.len()),
            Style::default().fg(DIM),
        )));
        for c in &comments {
            let cb = c.get("body").and_then(Value::as_str).unwrap_or("");
            let ca = c.get("created_at").and_then(Value::as_str).unwrap_or("");
            lines.push(Line::from(Span::styled(
                ca.to_string(),
                Style::default().fg(DIM),
            )));
            lines.extend(parse_markdown(cb, "  "));
        }
    }

    let inner_height = area.height.saturating_sub(4);
    let total = lines.len() as u16;
    let max_scroll = total.saturating_sub(inner_height);
    if app.detail_scroll > max_scroll {
        app.detail_scroll = max_scroll;
    }

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.detail_scroll, 0));
    f.render_widget(para, area);
}

fn handle_mouse(app: &mut App, db: &Db, m: MouseEvent) {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some((ci, row)) = hit_card(app, m.column, m.row) {
                let clicked_id = app.columns[ci].issues[row].id;
                app.selected_col = ci;
                app.columns[ci].state.select(Some(row));
                if app.detail_id == Some(clicked_id) {
                    app.close_detail();
                } else {
                    app.open_detail(db, clicked_id);
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if app.detail.is_some() && in_rect(app.detail_rect, m.column, m.row) {
                app.detail_scroll = app.detail_scroll.saturating_add(3);
            } else if let Some(ci) = hit_col(app, m.column, m.row) {
                app.selected_col = ci;
                app.move_item(1);
            }
        }
        MouseEventKind::ScrollUp => {
            if app.detail.is_some() && in_rect(app.detail_rect, m.column, m.row) {
                app.detail_scroll = app.detail_scroll.saturating_sub(3);
            } else if let Some(ci) = hit_col(app, m.column, m.row) {
                app.selected_col = ci;
                app.move_item(-1);
            }
        }
        _ => {}
    }
}

fn in_rect(r: Option<Rect>, x: u16, y: u16) -> bool {
    match r {
        Some(r) => x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height,
        None => false,
    }
}

fn hit_col(app: &App, mx: u16, my: u16) -> Option<usize> {
    for (ci, r) in app.column_rects.iter().enumerate() {
        if in_rect(Some(*r), mx, my) {
            return Some(ci);
        }
    }
    None
}

fn hit_card(app: &App, mx: u16, my: u16) -> Option<(usize, usize)> {
    for (ci, r) in app.column_rects.iter().enumerate() {
        if r.width < 2 || r.height < 2 {
            continue;
        }
        let inner_x = r.x + 1;
        let inner_w = r.width - 2;
        let inner_y = r.y + 1;
        let inner_h = r.height - 2;
        if mx < inner_x || mx >= inner_x + inner_w {
            continue;
        }
        if my < inner_y || my >= inner_y + inner_h {
            continue;
        }
        let offset = app.columns[ci].state.offset();
        let local = (my - inner_y) as usize + offset;
        if local < app.columns[ci].issues.len() {
            return Some((ci, local));
        }
    }
    None
}

fn parse_markdown(body: &str, prefix: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;
    let code_fg = Color::LightCyan;
    for raw in body.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            lines.push(Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled(raw.to_string(), Style::default().fg(code_fg)),
            ]));
            continue;
        }
        if let Some((level, rest)) = header_split(trimmed) {
            lines.push(Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled(rest.to_string(), header_style(level)),
            ]));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut spans = vec![
                Span::raw(prefix.to_string()),
                Span::styled("│ ", Style::default().fg(DIM)),
            ];
            for mut s in parse_inline(rest) {
                s.style = s
                    .style
                    .fg(MUTED)
                    .add_modifier(Modifier::ITALIC);
                spans.push(s);
            }
            lines.push(Line::from(spans));
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let indent = raw.len() - trimmed.len();
            let mut spans = vec![
                Span::raw(prefix.to_string()),
                Span::raw(" ".repeat(indent)),
                Span::styled("• ", Style::default().fg(MUTED)),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }
        let mut spans = vec![Span::raw(prefix.to_string())];
        spans.extend(parse_inline(raw));
        lines.push(Line::from(spans));
    }
    lines
}

fn header_split(s: &str) -> Option<(usize, &str)> {
    let mut level = 0;
    let bytes = s.as_bytes();
    while level < 6 && bytes.get(level) == Some(&b'#') {
        level += 1;
    }
    if level == 0 {
        return None;
    }
    if bytes.get(level) != Some(&b' ') {
        return None;
    }
    Some((level, &s[level + 1..]))
}

fn header_style(level: usize) -> Style {
    let color = match level {
        1 => Color::LightCyan,
        2 => Color::Cyan,
        3 => Color::LightBlue,
        _ => Color::Blue,
    };
    let mut st = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if level >= 4 {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    st
}

fn parse_inline(s: &str) -> Vec<Span<'static>> {
    let code_style = Style::default().fg(Color::LightCyan);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut bold = false;
    let mut italic = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let style_of = |b: bool, it: bool| {
        let mut st = Style::default();
        if b {
            st = st.add_modifier(Modifier::BOLD);
        }
        if it {
            st = st.add_modifier(Modifier::ITALIC);
        }
        st
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            if !buf.is_empty() {
                out.push(Span::styled(std::mem::take(&mut buf), style_of(bold, italic)));
            }
            let mut j = i + 1;
            let mut code = String::new();
            while j < chars.len() && chars[j] != '`' {
                code.push(chars[j]);
                j += 1;
            }
            if j < chars.len() {
                out.push(Span::styled(code, code_style));
                i = j + 1;
                continue;
            } else {
                buf.push('`');
                i += 1;
                continue;
            }
        }
        if c == '*' && chars.get(i + 1) == Some(&'*') {
            if !buf.is_empty() {
                out.push(Span::styled(std::mem::take(&mut buf), style_of(bold, italic)));
            }
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_') && chars.get(i + 1) != Some(&c) {
            if !buf.is_empty() {
                out.push(Span::styled(std::mem::take(&mut buf), style_of(bold, italic)));
            }
            italic = !italic;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    if !buf.is_empty() {
        out.push(Span::styled(buf, style_of(bold, italic)));
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let taken: String = s.chars().take(max - 1).collect();
    format!("{taken}…")
}
