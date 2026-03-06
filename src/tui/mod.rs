pub mod app;
pub mod display;
pub mod markdown;
pub mod render;

use crate::tui_client::TuiClient;
use anyhow::Result;
use crossterm::event::{Event, EventStream};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

pub async fn run_tui(port: u16, project_root: String) -> Result<()> {
    let client = TuiClient::new(port);

    // Wait for server health
    for _ in 0..50 {
        if client.health_check().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !client.health_check().await {
        anyhow::bail!("Server did not become healthy in time");
    }

    let (sse_rx, sse_abort) = client.subscribe_sse();
    let client = Arc::new(client);
    let mut terminal = setup_terminal()?;
    let mut app = app::App::new(client, sse_rx, project_root, port);

    let tick_rate = Duration::from_millis(50);
    let mut event_stream = EventStream::new();

    // Generate a session_id upfront so SSE events are filtered from the start.
    app.session_id = Some(format!(
        "tui-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8],
    ));

    // Kick off background fetch of skills/agents for autocomplete
    app.fetch_autocomplete_data();

    loop {
        // Drain all pending SSE events before rendering
        while let Ok(msg) = app.sse_rx.try_recv() {
            app.handle_sse(msg);
        }

        // Pick up auto-created session_id from spawned send_chat tasks
        if app.session_id.is_none() {
            if let Some(sid) = app.session_id_slot.lock().unwrap().take() {
                app.session_id = Some(sid);
            }
        }

        // Pick up autocomplete cache data from background fetch
        app.check_autocomplete_slots();

        terminal.draw(|f| app.render(f))?;

        tokio::select! {
            maybe_event = event_stream.next() => {
                if let Some(Ok(event)) = maybe_event {
                    match event {
                        Event::Key(key) => {
                            if app.handle_key(key)? {
                                sse_abort.abort();
                                restore_terminal(terminal)?;
                                return Ok(());
                            }
                        }
                        Event::Mouse(_) => {}
                        _ => {}
                    }
                }
            }
            Some(msg) = app.sse_rx.recv() => {
                app.handle_sse(msg);
            }
            _ = tokio::time::sleep(tick_rate) => {}
        }
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let (_, rows) = crossterm::terminal::size()?;
    let terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(rows),
        },
    )?;
    Ok(terminal)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    terminal.show_cursor()?;
    Ok(())
}
