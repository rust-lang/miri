use std::collections::VecDeque;
use std::io;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use std::time::{Duration, Instant};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Wrap};
use ratatui::{Frame, Terminal};

use super::channel::{CommandSender, StateReceiver};
use super::state::LocalKind;
use super::{DebuggerCommand, DebuggerState};

#[derive(Clone, Copy)]
enum FocusPane {
    Stack,
    Mir,
    Locals,
    Memory,
    Output,
}

impl FocusPane {
    fn next(self) -> Self {
        match self {
            FocusPane::Stack => FocusPane::Mir,
            FocusPane::Mir => FocusPane::Locals,
            FocusPane::Locals => FocusPane::Memory,
            FocusPane::Memory => FocusPane::Output,
            FocusPane::Output => FocusPane::Stack,
        }
    }
}

#[derive(Clone, Copy)]
enum RunMode {
    Step,
    Continue,
    RunToFrame,
    RunToMain,
    RunToEnd,
}

const THEME_ACCENT: Color = Color::Cyan;
const THEME_ACCENT_SOFT: Color = Color::LightCyan;
const THEME_DIM: Color = Color::DarkGray;
const THEME_BG: Color = Color::Black;
const THEME_OK: Color = Color::Green;
const THEME_WARN: Color = Color::Yellow;
const THEME_ERR: Color = Color::Red;
const CURSOR_BLINK_MS: u128 = 500;
const EVENT_POLL_MS: u64 = 100;
const HISTORY_CAPACITY: usize = 1000;

impl RunMode {
    fn as_str(self) -> &'static str {
        match self {
            RunMode::Step => "step",
            RunMode::Continue => "continue",
            RunMode::RunToFrame => "run-to-frame",
            RunMode::RunToMain => "run-to-main",
            RunMode::RunToEnd => "run-to-end",
        }
    }
}

#[derive(Default)]
struct UiScrollState {
    stack_index: usize,
    stack_hscroll: u16,
    mir_scroll: u16,
    mir_hscroll: u16,
    locals_scroll: usize,
    locals_hscroll: u16,
    memory_scroll: usize,
    memory_hscroll: u16,
    output_scroll: usize,
    output_hscroll: u16,
    status_hscroll: u16,
}

#[derive(Default)]
struct StackSearchState {
    query: String,
    editing: bool,
    matches: Vec<usize>,
    current_match: usize,
}

#[derive(Default)]
struct RunTargetState {
    editing: bool,
    query: String,
}

pub fn spawn_tui(state_rx: StateReceiver, command_tx: CommandSender) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("miri-debugger-tui".to_string())
        .spawn(move || {
            if let Err(err) = run_tui(state_rx, command_tx) {
                eprintln!("debugger TUI error: {err}");
            }
        })
        .expect("failed to spawn debugger TUI thread")
}

fn run_tui(state_rx: StateReceiver, command_tx: CommandSender) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = tui_loop(&mut terminal, state_rx, command_tx);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

fn tui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state_rx: StateReceiver,
    command_tx: CommandSender,
) -> io::Result<()> {
    let mut focus = FocusPane::Stack;
    let mut mode = RunMode::Step;
    let mut scroll = UiScrollState::default();
    let mut search = StackSearchState::default();
    let mut run_target = RunTargetState::default();
    let mut run_to_frame_target: Option<String> = None;
    let mut last_state: Option<DebuggerState> = None;
    let mut history: VecDeque<DebuggerState> = VecDeque::with_capacity(HISTORY_CAPACITY);
    let blink_epoch = Instant::now();

    while let Ok(state) = state_rx.recv() {
        history.push_back(state.clone());
        if history.len() > HISTORY_CAPACITY {
            history.pop_front();
        }

        last_state = Some(state.clone());
        refresh_stack_search(&state, &mut scroll, &mut search);
        if !state.stack_frames.is_empty() {
            scroll.stack_index = scroll.stack_index.min(state.stack_frames.len() - 1);
        } else {
            scroll.stack_index = 0;
        }

        let mut display_state = state.clone();
        let mut reverse_index: Option<usize> = None;

        if matches!(mode, RunMode::RunToFrame)
            && run_to_frame_target
                .as_ref()
                .is_some_and(|target| state_has_frame(&state, target))
        {
            mode = RunMode::Step;
            run_to_frame_target = None;
        }

        // In fast-forward mode, keep rendering every step without waiting for input.
        if (matches!(mode, RunMode::RunToMain) && !state.in_user_code)
            || matches!(mode, RunMode::RunToFrame)
            || matches!(mode, RunMode::RunToEnd)
        {
            let search_cursor_visible =
                search.editing && (blink_epoch.elapsed().as_millis() / CURSOR_BLINK_MS).is_multiple_of(2);
            terminal.draw(|frame| {
                render(
                    frame,
                    &state,
                    focus,
                    mode,
                    &scroll,
                    &search,
                    &run_target,
                    false,
                    search_cursor_visible,
                    false,
                    history.len(),
                )
            })?;

            // Still allow immediate quit while fast-forwarding.
            if event::poll(Duration::from_millis(0))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && key.code == KeyCode::Char('q')
            {
                let _ = command_tx.send(DebuggerCommand::Quit);
                return Ok(());
            }

            continue;
        }

        if matches!(mode, RunMode::RunToMain) && state.in_user_code {
            mode = RunMode::Step;
        }

        loop {
            let search_cursor_visible =
                search.editing && (blink_epoch.elapsed().as_millis() / CURSOR_BLINK_MS).is_multiple_of(2);
            terminal.draw(|frame| {
                render(
                    frame,
                    &display_state,
                    focus,
                    mode,
                    &scroll,
                    &search,
                    &run_target,
                    false,
                    search_cursor_visible,
                    reverse_index.is_some(),
                    history.len(),
                )
            })?;

            if !event::poll(Duration::from_millis(EVENT_POLL_MS))? {
                continue;
            }
            let ev = event::read()?;
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if run_target.editing {
                    match key.code {
                        KeyCode::Esc => {
                            run_target.editing = false;
                        }
                        KeyCode::Enter => {
                            let target = run_target.query.trim().to_string();
                            run_target.editing = false;
                            if !target.is_empty() {
                                reverse_index = None;
                                run_to_frame_target = Some(target.clone());
                                mode = RunMode::RunToFrame;
                                let _ = command_tx.send(DebuggerCommand::RunToFrame(target));
                                break;
                            }
                        }
                        KeyCode::Backspace => {
                            run_target.query.pop();
                        }
                        KeyCode::Char(c) => {
                            run_target.query.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }
                if search.editing {
                    match key.code {
                        KeyCode::Char('[') => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_sub(1);
                        }
                        KeyCode::Char(']') => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_add(1);
                        }
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('/') => {
                            search.editing = false;
                        }
                        KeyCode::Backspace => {
                            search.query.pop();
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                        KeyCode::Char(c) => {
                            search.query.push(c);
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                        _ => {}
                    }
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') => {
                        let _ = command_tx.send(DebuggerCommand::Quit);
                        return Ok(());
                    }
                    KeyCode::Char('/') => {
                        focus = FocusPane::Stack;
                        search.editing = true;
                    }
                    KeyCode::Char('P') => {
                        run_target.editing = true;
                        run_target.query.clear();
                    }
                    KeyCode::Char('p') => {
                        if let Some(target) = selected_stack_fn_name(&display_state, &search, &scroll) {
                            reverse_index = None;
                            run_to_frame_target = Some(target.clone());
                            mode = RunMode::RunToFrame;
                            let _ = command_tx.send(DebuggerCommand::RunToFrame(target));
                            break;
                        } else {
                            run_target.editing = true;
                            run_target.query.clear();
                        }
                    }
                    KeyCode::Char('.') => {
                        goto_next_search_match(&mut scroll, &mut search);
                    }
                    KeyCode::Char(',') => {
                        goto_prev_search_match(&mut scroll, &mut search);
                    }
                    KeyCode::Char('[') => {
                        scroll.status_hscroll = scroll.status_hscroll.saturating_sub(1);
                    }
                    KeyCode::Char(']') => {
                        scroll.status_hscroll = scroll.status_hscroll.saturating_add(1);
                    }
                    KeyCode::Esc => {
                        if !search.query.is_empty() {
                            search = StackSearchState::default();
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char(' ') => {
                        if let Some(idx) = reverse_index {
                            if idx + 1 < history.len() {
                                let next = idx + 1;
                                if let Some(snapshot) = history.get(next) {
                                    display_state = snapshot.clone();
                                    reverse_index = if next + 1 == history.len() {
                                        None
                                    } else {
                                        Some(next)
                                    };
                                    refresh_stack_search(&display_state, &mut scroll, &mut search);
                                }
                                continue;
                            }
                            reverse_index = None;
                            display_state = state.clone();
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                        mode = RunMode::Step;
                        let _ = command_tx.send(DebuggerCommand::StepOver);
                        break;
                    }
                    KeyCode::Char('b') => {
                        mode = RunMode::Step;
                        let next_index = match reverse_index {
                            Some(idx) => idx.saturating_sub(1),
                            None => history.len().saturating_sub(2),
                        };
                        if let Some(snapshot) = history.get(next_index) {
                            reverse_index = Some(next_index);
                            display_state = snapshot.clone();
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                    }
                    KeyCode::Char('c') => {
                        reverse_index = None;
                        run_to_frame_target = None;
                        mode = RunMode::Continue;
                        let _ = command_tx.send(DebuggerCommand::Continue);
                        break;
                    }
                    KeyCode::Char('e') => {
                        reverse_index = None;
                        run_to_frame_target = None;
                        mode = RunMode::RunToEnd;
                        let _ = command_tx.send(DebuggerCommand::RunToEnd);
                        break;
                    }
                    KeyCode::Char('m') => {
                        reverse_index = None;
                        run_to_frame_target = None;
                        mode = RunMode::RunToMain;
                        let _ = command_tx.send(DebuggerCommand::RunToMain);
                        break;
                    }
                    KeyCode::Tab => focus = focus.next(),
                    KeyCode::Up => match focus {
                        FocusPane::Stack => {
                            step_stack_selection(&display_state, &search, &mut scroll, false);
                        }
                        FocusPane::Mir => {
                            scroll.mir_scroll = scroll.mir_scroll.saturating_sub(1);
                        }
                        FocusPane::Locals => {
                            scroll.locals_scroll = scroll.locals_scroll.saturating_sub(1);
                        }
                        FocusPane::Memory => {
                            scroll.memory_scroll = scroll.memory_scroll.saturating_sub(1);
                        }
                        FocusPane::Output => {
                            scroll.output_scroll = scroll.output_scroll.saturating_sub(1);
                        }
                    },
                    KeyCode::Down => match focus {
                        FocusPane::Stack => {
                            step_stack_selection(&display_state, &search, &mut scroll, true);
                        }
                        FocusPane::Mir => {
                            scroll.mir_scroll = scroll.mir_scroll.saturating_add(1);
                        }
                        FocusPane::Locals => {
                            if !display_state.locals.is_empty() {
                                let max = display_state.locals.len() - 1;
                                scroll.locals_scroll = scroll.locals_scroll.saturating_add(1).min(max);
                            }
                        }
                        FocusPane::Memory => {
                            if !display_state.memory.is_empty() {
                                let max = display_state.memory.len() - 1;
                                scroll.memory_scroll = scroll.memory_scroll.saturating_add(1).min(max);
                            }
                        }
                        FocusPane::Output => {
                            if !display_state.output.is_empty() {
                                let max = display_state.output.len() - 1;
                                scroll.output_scroll = scroll.output_scroll.saturating_add(1).min(max);
                            }
                        }
                    },
                    KeyCode::Left => {
                        match focus {
                            FocusPane::Stack => {
                                scroll.stack_hscroll = scroll.stack_hscroll.saturating_sub(1);
                            }
                            FocusPane::Mir => {
                                scroll.mir_hscroll = scroll.mir_hscroll.saturating_sub(1);
                            }
                            FocusPane::Locals => {
                                scroll.locals_hscroll = scroll.locals_hscroll.saturating_sub(1);
                            }
                            FocusPane::Memory => {
                                scroll.memory_hscroll = scroll.memory_hscroll.saturating_sub(1);
                            }
                            FocusPane::Output => {
                                scroll.output_hscroll = scroll.output_hscroll.saturating_sub(1);
                            }
                        }
                    }
                    KeyCode::Right => {
                        match focus {
                            FocusPane::Stack => {
                                scroll.stack_hscroll = scroll.stack_hscroll.saturating_add(1);
                            }
                            FocusPane::Mir => {
                                scroll.mir_hscroll = scroll.mir_hscroll.saturating_add(1);
                            }
                            FocusPane::Locals => {
                                scroll.locals_hscroll = scroll.locals_hscroll.saturating_add(1);
                            }
                            FocusPane::Memory => {
                                scroll.memory_hscroll = scroll.memory_hscroll.saturating_add(1);
                            }
                            FocusPane::Output => {
                                scroll.output_hscroll = scroll.output_hscroll.saturating_add(1);
                            }
                        }
                    }
                    KeyCode::Char(c)
                        if c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '<' || c == '>' =>
                    {
                        run_target.editing = true;
                        run_target.query.clear();
                        run_target.query.push(c);
                    }
                    _ => {}
                }
            } else if let Event::Mouse(mouse) = ev {
                let size = terminal.size()?;
                if mouse.row == size.height.saturating_sub(1) {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_sub(1);
                        }
                        MouseEventKind::ScrollDown => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_add(1);
                        }
                        _ => {}
                    }
                    continue;
                }
                let area = Rect::new(0, 0, size.width, size.height);
                let hovered = pane_at(area, mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        focus = hovered;
                        scroll_up(&mut scroll, hovered);
                    }
                    MouseEventKind::ScrollDown => {
                        focus = hovered;
                        scroll_down(&display_state, &mut scroll, hovered);
                    }
                    _ => {}
                }
            }

        }
    }

    // Program is done; keep the final snapshot visible until the user explicitly quits.
    if let Some(state) = last_state {
        mode = RunMode::Step;
        search.editing = false;
        let mut display_state = state.clone();
        let mut reverse_index: Option<usize> = None;
        loop {
            let search_cursor_visible =
                search.editing && (blink_epoch.elapsed().as_millis() / CURSOR_BLINK_MS).is_multiple_of(2);
            terminal.draw(|frame| {
                render(
                    frame,
                    &display_state,
                    focus,
                    mode,
                    &scroll,
                    &search,
                    &run_target,
                    true,
                    search_cursor_visible,
                    reverse_index.is_some(),
                    history.len(),
                )
            })?;
            if !event::poll(Duration::from_millis(EVENT_POLL_MS))? {
                continue;
            }
            let ev = event::read()?;
            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if search.editing {
                    match key.code {
                        KeyCode::Char('[') => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_sub(1);
                        }
                        KeyCode::Char(']') => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_add(1);
                        }
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('/') => {
                            search.editing = false;
                        }
                        KeyCode::Backspace => {
                            search.query.pop();
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                        KeyCode::Char(c) => {
                            search.query.push(c);
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('b') => {
                        let next_index = match reverse_index {
                            Some(idx) => idx.saturating_sub(1),
                            None => history.len().saturating_sub(2),
                        };
                        if let Some(snapshot) = history.get(next_index) {
                            reverse_index = Some(next_index);
                            display_state = snapshot.clone();
                            refresh_stack_search(&display_state, &mut scroll, &mut search);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char(' ') => {
                        if let Some(idx) = reverse_index {
                            if idx + 1 < history.len() {
                                let next = idx + 1;
                                if let Some(snapshot) = history.get(next) {
                                    display_state = snapshot.clone();
                                    reverse_index = if next + 1 == history.len() {
                                        None
                                    } else {
                                        Some(next)
                                    };
                                    refresh_stack_search(&display_state, &mut scroll, &mut search);
                                }
                            }
                        }
                    }
                    KeyCode::Char('/') => {
                        focus = FocusPane::Stack;
                        search.editing = true;
                    }
                    KeyCode::Char('.') => {
                        goto_next_search_match(&mut scroll, &mut search);
                    }
                    KeyCode::Char(',') => {
                        goto_prev_search_match(&mut scroll, &mut search);
                    }
                    KeyCode::Char('[') => {
                        scroll.status_hscroll = scroll.status_hscroll.saturating_sub(1);
                    }
                    KeyCode::Char(']') => {
                        scroll.status_hscroll = scroll.status_hscroll.saturating_add(1);
                    }
                    KeyCode::Esc => {
                        if !search.query.is_empty() {
                            search = StackSearchState::default();
                        }
                    }
                    KeyCode::Tab => focus = focus.next(),
                    KeyCode::Up => match focus {
                        FocusPane::Stack => {
                            step_stack_selection(&display_state, &search, &mut scroll, false);
                        }
                        FocusPane::Mir => {
                            scroll.mir_scroll = scroll.mir_scroll.saturating_sub(1);
                        }
                        FocusPane::Locals => {
                            scroll.locals_scroll = scroll.locals_scroll.saturating_sub(1);
                        }
                        FocusPane::Memory => {
                            scroll.memory_scroll = scroll.memory_scroll.saturating_sub(1);
                        }
                        FocusPane::Output => {
                            scroll.output_scroll = scroll.output_scroll.saturating_sub(1);
                        }
                    },
                    KeyCode::Down => match focus {
                        FocusPane::Stack => {
                            step_stack_selection(&display_state, &search, &mut scroll, true);
                        }
                        FocusPane::Mir => {
                            scroll.mir_scroll = scroll.mir_scroll.saturating_add(1);
                        }
                        FocusPane::Locals => {
                            if !display_state.locals.is_empty() {
                                let max = display_state.locals.len() - 1;
                                scroll.locals_scroll = scroll.locals_scroll.saturating_add(1).min(max);
                            }
                        }
                        FocusPane::Memory => {
                            if !display_state.memory.is_empty() {
                                let max = display_state.memory.len() - 1;
                                scroll.memory_scroll = scroll.memory_scroll.saturating_add(1).min(max);
                            }
                        }
                        FocusPane::Output => {
                            if !display_state.output.is_empty() {
                                let max = display_state.output.len() - 1;
                                scroll.output_scroll = scroll.output_scroll.saturating_add(1).min(max);
                            }
                        }
                    },
                    KeyCode::Left => {
                        match focus {
                            FocusPane::Stack => {
                                scroll.stack_hscroll = scroll.stack_hscroll.saturating_sub(1);
                            }
                            FocusPane::Mir => {
                                scroll.mir_hscroll = scroll.mir_hscroll.saturating_sub(1);
                            }
                            FocusPane::Locals => {
                                scroll.locals_hscroll = scroll.locals_hscroll.saturating_sub(1);
                            }
                            FocusPane::Memory => {
                                scroll.memory_hscroll = scroll.memory_hscroll.saturating_sub(1);
                            }
                            FocusPane::Output => {
                                scroll.output_hscroll = scroll.output_hscroll.saturating_sub(1);
                            }
                        }
                    }
                    KeyCode::Right => {
                        match focus {
                            FocusPane::Stack => {
                                scroll.stack_hscroll = scroll.stack_hscroll.saturating_add(1);
                            }
                            FocusPane::Mir => {
                                scroll.mir_hscroll = scroll.mir_hscroll.saturating_add(1);
                            }
                            FocusPane::Locals => {
                                scroll.locals_hscroll = scroll.locals_hscroll.saturating_add(1);
                            }
                            FocusPane::Memory => {
                                scroll.memory_hscroll = scroll.memory_hscroll.saturating_add(1);
                            }
                            FocusPane::Output => {
                                scroll.output_hscroll = scroll.output_hscroll.saturating_add(1);
                            }
                        }
                    }
                    KeyCode::Char(c)
                        if c.is_ascii_alphanumeric() || c == '_' || c == ':' || c == '<' || c == '>' =>
                    {
                        run_target.editing = true;
                        run_target.query.clear();
                        run_target.query.push(c);
                    }
                    _ => {}
                }
            } else if let Event::Mouse(mouse) = ev {
                let size = terminal.size()?;
                if mouse.row == size.height.saturating_sub(1) {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_sub(1);
                        }
                        MouseEventKind::ScrollDown => {
                            scroll.status_hscroll = scroll.status_hscroll.saturating_add(1);
                        }
                        _ => {}
                    }
                    continue;
                }
                let area = Rect::new(0, 0, size.width, size.height);
                let hovered = pane_at(area, mouse.column, mouse.row);
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        focus = hovered;
                        scroll_up(&mut scroll, hovered);
                    }
                    MouseEventKind::ScrollDown => {
                        focus = hovered;
                        scroll_down(&display_state, &mut scroll, hovered);
                    }
                    _ => {}
                }
            }
        }
    } else {
        // No snapshot was received before the interpreter terminated. Keep a minimal
        // end screen open so users can still quit explicitly.
        let text = Paragraph::new("Program finished before first debugger snapshot. Press q to close.")
            .block(Block::default().title("Miri Debugger").borders(Borders::ALL))
            .wrap(Wrap { trim: true });

        loop {
            terminal.draw(|frame| frame.render_widget(text.clone(), frame.area()))?;
            let ev = event::read()?;
            if let Event::Key(key) = ev
                && key.kind == KeyEventKind::Press
                && key.code == KeyCode::Char('q')
            {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn render(
    frame: &mut Frame<'_>,
    state: &DebuggerState,
    focus: FocusPane,
    mode: RunMode,
    scroll: &UiScrollState,
    search: &StackSearchState,
    run_target: &RunTargetState,
    program_finished: bool,
    search_cursor_visible: bool,
    reverse_mode: bool,
    history_len: usize,
) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(outer[0]);

    render_stack_pane(frame, main[0], state, focus, scroll, search, search_cursor_visible);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(24),
            Constraint::Percentage(20),
            Constraint::Percentage(22),
        ])
        .split(main[1]);

    render_mir_pane(frame, right[0], state, focus, scroll);
    render_locals_pane(frame, right[1], state, focus, scroll);
    render_memory_pane(frame, right[2], state, focus, scroll);
    render_output_pane(frame, right[3], state, focus, scroll);
    render_status_bar(
        frame,
        outer[1],
        state,
        mode,
        focus,
        search,
        run_target,
        program_finished,
        reverse_mode,
        history_len,
        scroll,
    );
}

fn pane_at(area: Rect, x: u16, y: u16) -> FocusPane {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(outer[0]);
    if x < root[1].x {
        return FocusPane::Stack;
    }
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(24),
            Constraint::Percentage(20),
            Constraint::Percentage(22),
        ])
        .split(root[1]);
    if y < right[1].y {
        FocusPane::Mir
    } else if y < right[2].y {
        FocusPane::Locals
    } else if y < right[3].y {
        FocusPane::Memory
    } else {
        FocusPane::Output
    }
}

fn scroll_up(scroll: &mut UiScrollState, pane: FocusPane) {
    match pane {
        FocusPane::Stack => {
            scroll.stack_index = scroll.stack_index.saturating_sub(1);
        }
        FocusPane::Mir => {
            scroll.mir_scroll = scroll.mir_scroll.saturating_sub(1);
        }
        FocusPane::Locals => {
            scroll.locals_scroll = scroll.locals_scroll.saturating_sub(1);
        }
        FocusPane::Memory => {
            scroll.memory_scroll = scroll.memory_scroll.saturating_sub(1);
        }
        FocusPane::Output => {
            scroll.output_scroll = scroll.output_scroll.saturating_sub(1);
        }
    }
}

fn scroll_down(state: &DebuggerState, scroll: &mut UiScrollState, pane: FocusPane) {
    match pane {
        FocusPane::Stack => {
            if !state.stack_frames.is_empty() {
                let max = state.stack_frames.len() - 1;
                scroll.stack_index = scroll.stack_index.saturating_add(1).min(max);
            }
        }
        FocusPane::Mir => {
            scroll.mir_scroll = scroll.mir_scroll.saturating_add(1);
        }
        FocusPane::Locals => {
            let len = state
                .stack_frames
                .get(scroll.stack_index)
                .map(|f| f.locals.len())
                .unwrap_or(state.locals.len());
            if len > 0 {
                let max = len - 1;
                scroll.locals_scroll = scroll.locals_scroll.saturating_add(1).min(max);
            }
        }
        FocusPane::Memory => {
            if !state.memory.is_empty() {
                let max = state.memory.len() - 1;
                scroll.memory_scroll = scroll.memory_scroll.saturating_add(1).min(max);
            }
        }
        FocusPane::Output => {
            if !state.output.is_empty() {
                let max = state.output.len() - 1;
                scroll.output_scroll = scroll.output_scroll.saturating_add(1).min(max);
            }
        }
    }
}

fn pane_border_style(focus: FocusPane, pane: FocusPane) -> Style {
    if std::mem::discriminant(&focus) == std::mem::discriminant(&pane) {
        Style::default().fg(THEME_ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(THEME_DIM)
    }
}

fn render_stack_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebuggerState,
    focus: FocusPane,
    scroll: &UiScrollState,
    search: &StackSearchState,
    search_cursor_visible: bool,
) {
    let search_display = if search.editing {
        if search_cursor_visible {
            format!("{}|", search.query)
        } else {
            search.query.clone()
        }
    } else {
        search.query.clone()
    };

    let title = if search.editing && search.query.is_empty() {
        format!("Stack (thread {}) search:{}", state.current_thread.to_u32(), search_display)
    } else if search.query.is_empty() {
        format!("Stack (thread {})", state.current_thread.to_u32())
    } else {
        format!(
            "Stack (thread {}) search:{} [{}{}]",
            state.current_thread.to_u32(),
            search_display,
            search.matches.len()
            ,
            if search.editing { ", editing" } else { "" }
        )
    };

    let visible = visible_stack_indices(state, search);
    let items: Vec<_> = visible
        .iter()
        .copied()
        .map(|idx| {
            let info = &state.stack_frames[idx];
            let is_match = search.matches.contains(&idx);
            let first = hscroll_text(&format!("#{idx} {}", info.fn_name), scroll.stack_hscroll);
            let second = hscroll_text(
                &format!("{}:{}", info.source_file, info.line),
                scroll.stack_hscroll,
            );
            ListItem::new(vec![
                Line::from(first).style(Style::default().fg(THEME_ACCENT_SOFT)),
                Line::from(second).style(Style::default().fg(THEME_DIM)),
            ])
            .style(if is_match {
                Style::default().fg(THEME_WARN)
            } else {
                Style::default()
            })
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(pane_border_style(focus, FocusPane::Stack)),
        )
        .highlight_style(
            Style::default().bg(THEME_ACCENT).fg(THEME_BG).add_modifier(Modifier::BOLD),
        );

    let mut list_state = ListState::default();
    if !visible.is_empty() {
        let selected = visible
            .iter()
            .position(|idx| *idx == scroll.stack_index)
            .unwrap_or(0);
        list_state.select(Some(selected));
    }
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_mir_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebuggerState,
    focus: FocusPane,
    scroll: &UiScrollState,
) {
    let mut lines = vec![
        Line::from(format!(
            "{}:{}",
            state.current_location.source_file, state.current_location.line
        )),
        Line::from(""),
        Line::from(state.current_location.statement.clone())
            .style(Style::default().fg(THEME_ACCENT_SOFT)),
    ];

    lines.push(Line::from(""));
    lines.push(
        Line::from("CFG:")
            .style(Style::default().fg(THEME_ACCENT).add_modifier(Modifier::BOLD)),
    );
    lines.extend(state.cfg_lines.iter().map(|line| {
        let mut diagram = format!("bb{}", line.block);
        if line.successors.is_empty() {
            diagram.push_str(" ─┤ END");
        } else {
            let succs = line
                .successors
                .iter()
                .map(|s| format!("bb{s}"))
                .collect::<Vec<_>>()
                .join(" │ ");
            diagram.push_str(" ─┬─> ");
            diagram.push_str(&succs);
        }

        if line.is_current {
            Line::from(hscroll_text(&format!("▣ {}", diagram), scroll.mir_hscroll))
                .style(Style::default().fg(THEME_OK).add_modifier(Modifier::BOLD))
        } else {
            Line::from(hscroll_text(&format!("□ {}", diagram), scroll.mir_hscroll))
                .style(Style::default().fg(THEME_DIM))
        }
    }));

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .title("Current MIR")
                .borders(Borders::ALL)
                .border_style(pane_border_style(focus, FocusPane::Mir)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll.mir_scroll, scroll.mir_hscroll));

    frame.render_widget(paragraph, area);
}

fn render_locals_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebuggerState,
    focus: FocusPane,
    scroll: &UiScrollState,
) {
    let selected_locals = state
        .stack_frames
        .get(scroll.stack_index)
        .map(|f| f.locals.as_slice())
        .unwrap_or_else(|| state.locals.as_slice());

    let rows = selected_locals
        .iter()
        .skip(scroll.locals_scroll)
        .map(|local| {
            let value_style = match local.kind {
                LocalKind::Dead => Style::default().fg(THEME_DIM),
                LocalKind::Uninitialized => Style::default().fg(THEME_ERR).add_modifier(Modifier::BOLD),
                LocalKind::Pointer => Style::default().fg(THEME_WARN).add_modifier(Modifier::BOLD),
                LocalKind::Initialized => Style::default().fg(THEME_OK),
            };
            let name_style = if local.kind == LocalKind::Dead {
                Style::default().fg(THEME_DIM)
            } else {
                Style::default().fg(THEME_ACCENT_SOFT)
            };
            Row::new([
                Cell::from(local.name.clone()).style(name_style),
                Cell::from(local.ty.clone()).style(Style::default().fg(THEME_DIM)),
                Cell::from(hscroll_text(&local.value, scroll.locals_hscroll)).style(value_style),
            ])
        });

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Percentage(30),
            Constraint::Percentage(70),
        ],
    )
    .header(
        Row::new(["Local", "Type", "Value"]).style(
            Style::default().fg(THEME_ACCENT).add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .title("Locals")
            .borders(Borders::ALL)
            .border_style(pane_border_style(focus, FocusPane::Locals)),
    );

    frame.render_widget(table, area);
}

fn render_memory_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebuggerState,
    focus: FocusPane,
    scroll: &UiScrollState,
) {
    let items: Vec<ListItem<'_>> = state
        .memory
        .iter()
        .skip(scroll.memory_scroll)
        .map(|mem| {
            let live = mem.detail.contains("live") || mem.name.contains("ptr");
            let blocks = if live { "■■■■■■" } else { "□□□□□□" };
            let block_style = if live {
                Style::default().fg(THEME_ACCENT)
            } else {
                Style::default().fg(THEME_DIM)
            };
            let line = format!(
                "{}  {} => {}",
                blocks,
                hscroll_text(&mem.name, scroll.memory_hscroll),
                hscroll_text(&mem.detail, scroll.memory_hscroll)
            );
            ListItem::new(Line::from(line).style(block_style))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Memory")
            .borders(Borders::ALL)
            .border_style(pane_border_style(focus, FocusPane::Memory)),
    );

    frame.render_widget(list, area);
}

fn render_status_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebuggerState,
    mode: RunMode,
    focus: FocusPane,
    search: &StackSearchState,
    run_target: &RunTargetState,
    program_finished: bool,
    reverse_mode: bool,
    history_len: usize,
    scroll: &UiScrollState,
) {
    let focus_name = match focus {
        FocusPane::Stack => "stack",
        FocusPane::Mir => "mir",
        FocusPane::Locals => "locals",
        FocusPane::Memory => "memory",
        FocusPane::Output => "output",
    };
    let search_text = if search.editing && search.query.is_empty() {
        "search=editing".to_string()
    } else if search.editing {
        format!("search=/{}, matches={} (editing)", search.query, search.matches.len())
    } else if search.query.is_empty() {
        "search=off".to_string()
    } else {
        format!("search=/{}, matches={}", search.query, search.matches.len())
    };
    let keys_text = if run_target.editing {
        "keys: type function name  enter run-to-frame  esc cancel  backspace delete"
    } else if search.editing {
        "keys: type to filter stack  enter/esc// exit search  backspace delete  [ ] scroll-cmds  q quit"
    } else if program_finished {
        "keys: q quit  / search  . next  , prev  b step-back  [ ] scroll-cmds  esc clear  tab switch  arrows scroll"
    } else {
        "keys: n/space step  b step-back  p run-to-selected  P run-to-name  c continue  m run-to-main  e run-to-end  / search  . next  , prev  [ ] scroll-cmds  q quit  tab switch  arrows scroll"
    };
    let finished_text = if program_finished { "  status=finished" } else { "" };
    let mode_text = if reverse_mode { "reverse" } else { mode.as_str() };
    let target_text = if run_target.editing {
        format!("  target={}|", run_target.query)
    } else {
        String::new()
    };
    let text = format!(
        "mode={}  steps={}  thread={}  focus={}  history={}/{}  {}{}{}  {}",
        mode_text,
        state.step_count,
        state.current_thread.to_u32(),
        focus_name,
        history_len,
        HISTORY_CAPACITY,
        search_text,
        finished_text,
        target_text,
        keys_text,
    );
    let bar = Paragraph::new(text)
        .style(Style::default().fg(THEME_BG).bg(THEME_ACCENT).add_modifier(Modifier::BOLD))
        .scroll((0, scroll.status_hscroll));
    frame.render_widget(bar, area);
}

fn refresh_stack_search(state: &DebuggerState, scroll: &mut UiScrollState, search: &mut StackSearchState) {
    if search.query.is_empty() {
        search.matches.clear();
        search.current_match = 0;
        return;
    }

    let query = search.query.to_ascii_lowercase();
    search.matches = state
        .stack_frames
        .iter()
        .enumerate()
        .filter_map(|(idx, frame)| {
            let hay = format!("{} {}:{}", frame.fn_name, frame.source_file, frame.line).to_ascii_lowercase();
            if hay.contains(&query) {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    if search.matches.is_empty() {
        search.current_match = 0;
        return;
    }

    if search.current_match >= search.matches.len() {
        search.current_match = 0;
    }
    scroll.stack_index = search.matches[search.current_match];
}

fn visible_stack_indices(state: &DebuggerState, search: &StackSearchState) -> Vec<usize> {
    if search.query.is_empty() {
        (0..state.stack_frames.len()).collect()
    } else {
        search.matches.clone()
    }
}

fn step_stack_selection(
    state: &DebuggerState,
    search: &StackSearchState,
    scroll: &mut UiScrollState,
    forward: bool,
) {
    let visible = visible_stack_indices(state, search);
    if visible.is_empty() {
        return;
    }

    let current_pos = visible
        .iter()
        .position(|idx| *idx == scroll.stack_index)
        .unwrap_or(0);
    let next_pos = if forward {
        (current_pos + 1).min(visible.len() - 1)
    } else {
        current_pos.saturating_sub(1)
    };
    scroll.stack_index = visible[next_pos];
}

fn state_has_frame(state: &DebuggerState, target: &str) -> bool {
    let target_lc = target.to_ascii_lowercase();
    state
        .stack_frames
        .iter()
        .any(|frame| frame.fn_name.to_ascii_lowercase().contains(&target_lc))
}

fn selected_stack_fn_name(
    state: &DebuggerState,
    search: &StackSearchState,
    scroll: &UiScrollState,
) -> Option<String> {
    let visible = visible_stack_indices(state, search);
    if visible.is_empty() {
        return None;
    }
    let idx = if visible.contains(&scroll.stack_index) {
        scroll.stack_index
    } else {
        visible[0]
    };
    state.stack_frames.get(idx).map(|f| f.fn_name.clone())
}

fn goto_next_search_match(scroll: &mut UiScrollState, search: &mut StackSearchState) {
    if search.matches.is_empty() {
        return;
    }
    search.current_match = (search.current_match + 1) % search.matches.len();
    scroll.stack_index = search.matches[search.current_match];
}

fn goto_prev_search_match(scroll: &mut UiScrollState, search: &mut StackSearchState) {
    if search.matches.is_empty() {
        return;
    }
    if search.current_match == 0 {
        search.current_match = search.matches.len() - 1;
    } else {
        search.current_match -= 1;
    }
    scroll.stack_index = search.matches[search.current_match];
}

fn hscroll_text(text: &str, offset: u16) -> String {
    text.chars().skip(usize::from(offset)).collect()
}

fn render_output_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &DebuggerState,
    focus: FocusPane,
    scroll: &UiScrollState,
) {
    let items: Vec<ListItem<'_>> = state
        .output
        .iter()
        .skip(scroll.output_scroll)
        .flat_map(|entry| {
            entry.text.lines().map(move |line| {
                let style = if entry.is_stderr {
                    Style::default().fg(THEME_ERR)
                } else {
                    Style::default().fg(THEME_ACCENT_SOFT)
                };
                ListItem::new(Line::from(hscroll_text(line, scroll.output_hscroll)).style(style))
            })
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title("Output")
            .borders(Borders::ALL)
            .border_style(pane_border_style(focus, FocusPane::Output)),
    );
    frame.render_widget(list, area);
}
