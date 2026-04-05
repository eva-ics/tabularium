//! Chat TUI for `tb chat` using `ratatui` + `tui-textarea`.

use std::io::{Write, stdout};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    self, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tabularium::rpc::Client as RpcClient;
use tabularium::ws::{Client, RecvMessage};
use tempfile::Builder;
use tui_textarea::TextArea;

use crate::chat_markdown::markdown_transcript_text;
use crate::execute::{
    BoxErr, ChatSubmitAction, ExecuteOpts, ExecuteOutcome, chat_edit_full_document,
    parse_chat_submit, run_editor, shell_pager_always,
};
use crate::render::mad_skin;

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    keyboard_enhancement: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self, BoxErr> {
        enable_raw_mode().map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;

        let mut stdout = stdout();
        if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, Hide) {
            let _ = disable_raw_mode();
            return Err(format!("terminal: {e}").into());
        }

        let keyboard_enhancement = execute!(
            stdout,
            PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
        )
        .is_ok();

        let backend = CrosstermBackend::new(stdout);
        let terminal =
            Terminal::new(backend).map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;

        Ok(Self {
            terminal,
            keyboard_enhancement,
        })
    }

    fn suspend(&mut self) -> Result<(), BoxErr> {
        let backend = self.terminal.backend_mut();
        if self.keyboard_enhancement {
            execute!(backend, PopKeyboardEnhancementFlags)
                .map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;
        }
        execute!(backend, Show, DisableBracketedPaste, LeaveAlternateScreen)
            .map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;
        disable_raw_mode().map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;
        Ok(())
    }

    fn resume(&mut self) -> Result<(), BoxErr> {
        enable_raw_mode().map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;

        let backend = self.terminal.backend_mut();
        execute!(backend, EnterAlternateScreen, EnableBracketedPaste, Hide)
            .map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;
        if self.keyboard_enhancement {
            execute!(
                backend,
                PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
            )
            .map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;
        }
        self.terminal
            .clear()
            .map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let backend = self.terminal.backend_mut();
        if self.keyboard_enhancement {
            let _ = execute!(backend, PopKeyboardEnhancementFlags);
        }
        let _ = execute!(backend, Show, DisableBracketedPaste, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn apply_ws_to_transcript(transcript: &mut String, msg: &RecvMessage) -> Result<(), BoxErr> {
    match msg {
        RecvMessage::Append { data: Some(d), .. } => transcript.push_str(d),
        RecvMessage::Reset { data: Some(d), .. } => {
            transcript.clear();
            transcript.push_str(d);
        }
        RecvMessage::Append { data: None, .. } | RecvMessage::Reset { data: None, .. } => {}
        RecvMessage::Error { message } => {
            return Err(message
                .clone()
                .unwrap_or_else(|| "unknown error".into())
                .into());
        }
        RecvMessage::Unknown { op } => {
            return Err(format!("unexpected ws op: {op}").into());
        }
    }
    Ok(())
}

fn new_textarea() -> TextArea<'static> {
    let mut textarea = TextArea::default();
    textarea.set_style(Style::default().fg(Color::White));
    textarea.set_cursor_style(
        Style::default()
            .bg(Color::Cyan)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    );
    textarea.set_cursor_line_style(Style::default());
    textarea
}

fn textarea_message(textarea: &TextArea<'_>) -> String {
    textarea.lines().join("\n")
}

fn is_ctrl_c(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c' | 'C')) && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_ctrl_d(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('d' | 'D')) && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_send_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Enter && key.modifiers.is_empty()
}

fn is_ctrl_e(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('e' | 'E'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

fn is_newline_key(key: KeyEvent) -> bool {
    (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT))
        || (matches!(key.code, KeyCode::Char('o' | 'O'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT))
}

fn is_scroll_up_key(key: KeyEvent) -> bool {
    key.code == KeyCode::PageUp
        || (key.code == KeyCode::Up && key.modifiers.contains(KeyModifiers::SHIFT))
        || (matches!(key.code, KeyCode::Char('b' | 'B'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT))
}

fn is_scroll_down_key(key: KeyEvent) -> bool {
    key.code == KeyCode::PageDown
        || (key.code == KeyCode::Down && key.modifiers.contains(KeyModifiers::SHIFT))
        || (matches!(key.code, KeyCode::Char('f' | 'F'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT))
}

fn scroll_indicator_color_ok() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

fn max_display_scroll(rows_len: usize, vis_h: usize) -> usize {
    rows_len.saturating_sub(vis_h)
}

/// Live tail vs history browsing: local send pins; remote append preserves a detached viewport.
struct ChatScrollState {
    pinned_bottom: bool,
    scroll_off: usize,
}

impl ChatScrollState {
    fn new() -> Self {
        Self {
            pinned_bottom: true,
            scroll_off: 0,
        }
    }

    fn pin_to_bottom(&mut self) {
        self.pinned_bottom = true;
    }

    fn effective_offset(&self, rows_len: usize, vis_h: usize) -> usize {
        let max_s = max_display_scroll(rows_len, vis_h);
        if self.pinned_bottom {
            max_s
        } else {
            self.scroll_off.min(max_s)
        }
    }

    fn clamp_when_detached(&mut self, rows_len: usize, vis_h: usize) {
        if !self.pinned_bottom {
            let max_s = max_display_scroll(rows_len, vis_h);
            self.scroll_off = self.scroll_off.min(max_s);
        }
    }

    fn page_up(&mut self, vis_h: usize) {
        self.pinned_bottom = false;
        self.scroll_off = self.scroll_off.saturating_sub(vis_h.max(1));
    }

    fn page_down(&mut self, rows_len: usize, vis_h: usize) {
        let vis = vis_h.max(1);
        let max_s = max_display_scroll(rows_len, vis_h);
        if self.pinned_bottom {
            return;
        }
        self.scroll_off = (self.scroll_off + vis).min(max_s);
        if rows_len <= vis_h || self.scroll_off >= max_s {
            self.pinned_bottom = true;
        }
    }

    fn on_remote_content_changed(&mut self, was_pinned: bool) {
        if was_pinned {
            self.pinned_bottom = true;
        }
    }
}

enum StatusTone {
    Info,
    Error,
}

struct StatusLine {
    text: String,
    tone: StatusTone,
}

impl StatusLine {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: StatusTone::Info,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: StatusTone::Error,
        }
    }

    fn render(&self) -> Line<'static> {
        let style = match self.tone {
            StatusTone::Info => Style::default().fg(Color::DarkGray),
            StatusTone::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        };
        Line::from(Span::styled(self.text.clone(), style))
    }
}

fn replace_textarea(textarea: &mut TextArea<'static>, text: &str) {
    let mut next = new_textarea();
    if !text.is_empty() {
        next.insert_str(text);
    }
    *textarea = next;
}

fn normalize_editor_text(mut text: String) -> String {
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    text
}

fn edit_text_in_editor(session: &mut TerminalSession, initial: &str) -> Result<String, BoxErr> {
    let mut tmp = Builder::new()
        .prefix("tb-chat-")
        .suffix(".md")
        .tempfile()
        .map_err(|e| -> BoxErr { e.into() })?;
    tmp.write_all(initial.as_bytes())
        .map_err(|e| -> BoxErr { e.into() })?;
    tmp.flush().map_err(|e| -> BoxErr { e.into() })?;
    let tmp_path = tmp.path().to_path_buf();

    session.suspend()?;
    let edit_result = run_editor(&tmp_path);
    let resume_result = session.resume();
    resume_result?;
    edit_result.map_err(|e| -> BoxErr { e.into() })?;

    let edited = std::fs::read_to_string(tmp_path).map_err(|e| -> BoxErr { e.into() })?;
    Ok(normalize_editor_text(edited))
}

fn wrap_transcript_lines(transcript: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if transcript.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    for logical in transcript.split('\n') {
        if logical.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut rest = logical;
        while !rest.is_empty() {
            let row: String = rest.chars().take(width).collect();
            let row_bytes = row.len();
            lines.push(row);
            rest = &rest[row_bytes..];
        }
    }
    lines
}

fn prompt_target(host: &str, path: &str) -> String {
    format!("{host}/{}", path.trim_start_matches('/'))
}

fn canonical_prompt(host: &str, path: &str, nickname: &str) -> String {
    format!("tb {} {nickname}", prompt_target(host, path))
}

fn prompt_banner(host: &str, path: &str, nickname: &str, width: usize) -> Line<'static> {
    let identity = canonical_prompt(host, path, nickname);
    let base = format!("━━ {identity} ");
    let fill = "━".repeat(width.saturating_sub(base.chars().count()));

    Line::from(vec![
        Span::styled("━━ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "tb",
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(prompt_target(host, path), Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(
            nickname.to_owned(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(fill, Style::default().fg(Color::DarkGray)),
    ])
}

fn transcript_all_lines(
    transcript: &str,
    width: usize,
    raw_transcript: bool,
) -> Vec<Line<'static>> {
    let width = width.max(1);
    if raw_transcript {
        if transcript.is_empty() {
            return vec![Line::from("")];
        }
        return wrap_transcript_lines(transcript, width)
            .into_iter()
            .map(Line::from)
            .collect();
    }
    let skin = mad_skin();
    markdown_transcript_text(transcript, width.max(3), &skin)
        .lines
        .into_iter()
        .collect()
}

/// Approximate transcript viewport (full terminal width × remaining rows above input banner).
fn chat_viewport_metrics(
    transcript: &str,
    raw: bool,
    textarea: &TextArea<'_>,
    status: bool,
) -> (usize, usize) {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    let vis_w = usize::from(cols).max(1);
    let r = usize::from(rows);
    let input_h = textarea.lines().len().max(1).min(r.saturating_sub(4));
    let st = usize::from(status);
    let vis_h = r.saturating_sub(input_h + 1 + st + 1).max(1);
    let n = transcript_all_lines(transcript, vis_w, raw).len();
    (n, vis_h)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn draw(
    session: &mut TerminalSession,
    transcript: &str,
    textarea: &mut TextArea<'static>,
    host: &str,
    path: &str,
    nickname: &str,
    status: Option<&StatusLine>,
    raw_transcript: bool,
    scroll: &mut ChatScrollState,
) -> Result<(), BoxErr> {
    session
        .terminal
        .draw(|frame| {
            let area = frame.area();
            let input_height = u16::try_from(textarea.lines().len().max(1)).unwrap_or(u16::MAX);
            let status_height = u16::from(status.is_some());
            let input_height =
                input_height.clamp(1, area.height.saturating_sub(2 + status_height).max(1));
            let vertical = if status.is_some() {
                Layout::vertical([
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(input_height),
                ])
            } else {
                Layout::vertical([
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(input_height),
                ])
            };

            let (transcript_area, status_area, banner_area, input_area) = if status.is_some() {
                let [transcript_area, status_area, banner_area, input_area] = vertical.areas(area);
                (transcript_area, Some(status_area), banner_area, input_area)
            } else {
                let [transcript_area, banner_area, input_area] = vertical.areas(area);
                (transcript_area, None, banner_area, input_area)
            };
            let [prompt_area, editor_area] =
                Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).areas(input_area);

            let show_scroll_ind = !scroll.pinned_bottom && scroll_indicator_color_ok();
            if show_scroll_ind {
                let [ind_row, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
                    .areas(transcript_area);
                let body_w = body.width.max(1) as usize;
                let body_h = body.height.max(1) as usize;
                let all_rows = transcript_all_lines(transcript, body_w, raw_transcript);
                scroll.clamp_when_detached(all_rows.len(), body_h);
                let off = scroll.effective_offset(all_rows.len(), body_h);
                let total = all_rows.len().max(1);
                let label = format!("{}/{}", off.saturating_add(1).min(total), total);
                let w = ind_row.width.max(1) as usize;
                let pad = w.saturating_sub(label.chars().count());
                let ind_line = Line::from(vec![
                    Span::raw(" ".repeat(pad)),
                    Span::styled(
                        label,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]);
                frame.render_widget(Paragraph::new(ind_line), ind_row);

                let end = (off + body_h).min(all_rows.len());
                let window: Vec<Line<'static>> = if off < all_rows.len() && off < end {
                    all_rows[off..end].to_vec()
                } else {
                    vec![]
                };
                let visible = if window.is_empty() {
                    Text::from(vec![Line::from(""); body_h.max(1)])
                } else {
                    Text::from(window)
                };
                frame.render_widget(Paragraph::new(visible), body);
            } else {
                let vis_w = transcript_area.width.max(1) as usize;
                let vis_h = transcript_area.height.max(1) as usize;
                let all_rows = transcript_all_lines(transcript, vis_w, raw_transcript);
                scroll.clamp_when_detached(all_rows.len(), vis_h);
                let off = scroll.effective_offset(all_rows.len(), vis_h);
                let end = (off + vis_h).min(all_rows.len());
                let window: Vec<Line<'static>> = if off < all_rows.len() && off < end {
                    all_rows[off..end].to_vec()
                } else {
                    vec![]
                };
                let visible = if window.is_empty() {
                    Text::from(vec![Line::from(""); vis_h.max(1)])
                } else {
                    Text::from(window)
                };
                frame.render_widget(Paragraph::new(visible), transcript_area);
            }

            if let (Some(status), Some(status_area)) = (status, status_area) {
                frame.render_widget(Paragraph::new(status.render()), status_area);
            }
            frame.render_widget(
                Paragraph::new(prompt_banner(
                    host,
                    path,
                    nickname,
                    banner_area.width.max(1) as usize,
                )),
                banner_area,
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "→",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                ])),
                prompt_area,
            );
            textarea.remove_block();
            frame.render_widget(&*textarea, editor_area);
        })
        .map_err(|e| -> BoxErr { format!("terminal: {e}").into() })?;

    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn run_chat(
    mut ws: Client,
    rpc: &RpcClient,
    path: &str,
    nickname: &str,
    host: &str,
    opts: ExecuteOpts,
) -> Result<ExecuteOutcome, BoxErr> {
    let mut session = TerminalSession::enter()?;

    let Some(first) = ws.recv().await? else {
        let _ = ws.close().await;
        return Ok(ExecuteOutcome::Ok);
    };

    let mut transcript = String::new();
    apply_ws_to_transcript(&mut transcript, &first)?;

    let mut textarea = new_textarea();
    let mut nickname = nickname.to_owned();
    let mut status = None;
    let mut scroll = ChatScrollState::new();
    let mut reader = Box::pin(EventStream::new());
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    let raw_transcript = opts.raw;
    draw(
        &mut session,
        &transcript,
        &mut textarea,
        host,
        path,
        &nickname,
        status.as_ref(),
        raw_transcript,
        &mut scroll,
    )?;

    loop {
        tokio::select! {
            event = reader.next() => {
                let Some(event) = event else {
                    break;
                };

                match event {
                    Ok(Event::Paste(text)) => {
                        textarea.insert_str(text);
                    }
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Release => {}
                    Ok(Event::Key(key))
                        if key.kind != KeyEventKind::Release
                            && key.code == KeyCode::Esc
                            && !scroll.pinned_bottom =>
                    {
                        scroll.pin_to_bottom();
                        status = None;
                    }
                    Ok(Event::Key(key)) if is_scroll_up_key(key) => {
                        let (rows_len, vis_h) = chat_viewport_metrics(
                            &transcript,
                            raw_transcript,
                            &textarea,
                            status.is_some(),
                        );
                        scroll.page_up(vis_h);
                        scroll.clamp_when_detached(rows_len, vis_h);
                        status = None;
                    }
                    Ok(Event::Key(key)) if is_scroll_down_key(key) => {
                        let (rows_len, vis_h) = chat_viewport_metrics(
                            &transcript,
                            raw_transcript,
                            &textarea,
                            status.is_some(),
                        );
                        scroll.page_down(rows_len, vis_h);
                        status = None;
                    }
                    Ok(Event::Key(key)) if is_ctrl_c(key) => {
                        let _ = ws.close().await;
                        return Ok(ExecuteOutcome::Interrupted);
                    }
                    Ok(Event::Key(key)) if is_ctrl_d(key) => {
                        if textarea_message(&textarea).trim().is_empty() {
                            break;
                        }
                        textarea.input(key);
                        status = None;
                    }
                    Ok(Event::Key(key)) if is_ctrl_e(key) => {
                        match edit_text_in_editor(&mut session, &textarea_message(&textarea)) {
                            Ok(edited) => {
                                replace_textarea(&mut textarea, &edited);
                                status = None;
                            }
                            Err(e) => {
                                status = Some(StatusLine::error(format!("editor: {e}")));
                            }
                        }
                    }
                    Ok(Event::Key(key)) if is_send_key(key) => {
                        match parse_chat_submit(&textarea_message(&textarea)) {
                            Ok(ChatSubmitAction::Ignore) => {
                                status = None;
                            }
                            Ok(ChatSubmitAction::Exit) => break,
                            Ok(ChatSubmitAction::Edit) => {
                                match edit_text_in_editor(&mut session, "") {
                                    Ok(edited) => {
                                        replace_textarea(&mut textarea, &edited);
                                        status = None;
                                    }
                                    Err(e) => {
                                        status = Some(StatusLine::error(format!("editor: {e}")));
                                    }
                                }
                            }
                            Ok(ChatSubmitAction::EditFullDocument { path: target }) => {
                                let doc_path = target.as_deref().unwrap_or(path);
                                if let Err(e) = session.suspend() {
                                    status = Some(StatusLine::error(format!("terminal: {e}")));
                                } else {
                                    let edit_res = chat_edit_full_document(rpc, doc_path).await;
                                    if let Err(e) = session.resume() {
                                        status = Some(StatusLine::error(format!("terminal: {e}")));
                                    } else {
                                        match edit_res {
                                            Ok(()) => status = None,
                                            Err(e) => {
                                                status =
                                                    Some(StatusLine::error(format!("edit document: {e}")));
                                            }
                                        }
                                    }
                                }
                                textarea = new_textarea();
                            }
                            Ok(ChatSubmitAction::History) => {
                                if let Err(e) = session.suspend() {
                                    status = Some(StatusLine::error(format!("terminal: {e}")));
                                } else {
                                    let pager_res: Result<(), BoxErr> =
                                        match rpc.get_document(path).await {
                                            Ok(body) => shell_pager_always(body.content()),
                                            Err(e) => Err(e.to_string().into()),
                                        };
                                    if let Err(e) = session.resume() {
                                        status = Some(StatusLine::error(format!("terminal: {e}")));
                                    } else {
                                        match pager_res {
                                            Ok(()) => status = None,
                                            Err(e) => {
                                                status =
                                                    Some(StatusLine::error(format!("history: {e}")));
                                            }
                                        }
                                    }
                                }
                                textarea = new_textarea();
                            }
                            Ok(ChatSubmitAction::ChangeNick(next)) => {
                                nickname = next;
                                textarea = new_textarea();
                                status = Some(StatusLine::info(format!(
                                    "nickname changed to {nickname}"
                                )));
                            }
                            Ok(ChatSubmitAction::Send(message)) => {
                                match ws.say(path, &nickname, &message).await {
                                    Ok(()) => {
                                        textarea = new_textarea();
                                        scroll.pin_to_bottom();
                                        status = None;
                                    }
                                    Err(e) => {
                                        status = Some(StatusLine::error(format!("say: {e}")));
                                    }
                                }
                            }
                            Err(message) => {
                                status = Some(StatusLine::error(message));
                            }
                        }
                    }
                    Ok(Event::Key(key)) if is_newline_key(key) => {
                        textarea.insert_newline();
                        status = None;
                    }
                    Ok(Event::Key(key)) => {
                        textarea.input(key);
                        status = None;
                    }
                    Ok(_) => {}
                    Err(e) => return Err(format!("terminal input: {e}").into()),
                }

                draw(
                    &mut session,
                    &transcript,
                    &mut textarea,
                    host,
                    path,
                    &nickname,
                    status.as_ref(),
                    raw_transcript,
                    &mut scroll,
                )?;
            }
            res = ws.recv() => {
                let Some(msg) = res? else {
                    break;
                };
                let was_pinned = scroll.pinned_bottom;
                apply_ws_to_transcript(&mut transcript, &msg)?;
                scroll.on_remote_content_changed(was_pinned);
                draw(
                    &mut session,
                    &transcript,
                    &mut textarea,
                    host,
                    path,
                    &nickname,
                    status.as_ref(),
                    raw_transcript,
                    &mut scroll,
                )?;
            }
            _ = &mut interrupted => {
                let _ = ws.close().await;
                return Ok(ExecuteOutcome::Interrupted);
            }
        }
    }

    let _ = ws.close().await;
    Ok(ExecuteOutcome::Ok)
}

#[cfg(test)]
mod tests {
    use super::{canonical_prompt, wrap_transcript_lines};

    #[test]
    fn preserves_blank_lines_for_markdown_chat() {
        let transcript = "## nick\n\nhello\n\n";
        let lines = wrap_transcript_lines(transcript, 80);
        assert_eq!(lines, vec!["## nick", "", "hello", "", ""]);
    }

    #[test]
    fn wraps_long_transcript_rows() {
        let lines = wrap_transcript_lines("abcdef", 3);
        assert_eq!(lines, vec!["abc", "def"]);
    }

    #[test]
    fn canonical_prompt_keeps_full_chat_identity() {
        assert_eq!(
            canonical_prompt("127.0.0.1:3050", "CAT/DOC", "gigabito"),
            "tb 127.0.0.1:3050/CAT/DOC gigabito"
        );
    }

    #[test]
    fn canonical_prompt_collapses_host_path_boundary_slashes() {
        assert_eq!(
            canonical_prompt("127.0.0.1:3050", "/tabularium/meetings/debs", "gigabito"),
            "tb 127.0.0.1:3050/tabularium/meetings/debs gigabito"
        );
    }
}
