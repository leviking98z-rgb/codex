use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn session_alias_overrides_display_names_but_not_saved_thread_titles() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(Some("gpt-5.2")).await;
    chat.transcript.active_cell = None;
    chat.show_welcome_banner = false;
    let thread_id = ThreadId::new();
    chat.thread_id = Some(thread_id);
    chat.thread_name = Some("Saved project title".to_string());
    chat.session_alias
        .set_thread(thread_id, chat.app_event_tx.clone());
    chat.local_settings.tui.status_line = Some(vec![
        "model-with-reasoning".to_string(),
        "thread-title".to_string(),
    ]);
    chat.local_settings.tui.terminal_title = Some(vec!["thread-name".to_string()]);
    chat.session_alias
        .reader()
        .update(Some("mail-agent".to_string()));
    chat.on_session_alias_updated(thread_id);
    assert_eq!(
        chat.status_surface_preview_value_for_item(StatusSurfacePreviewItem::AppName),
        Some("mail-agent".to_string())
    );
    assert_eq!(
        chat.terminal_title_value_for_item(TerminalTitleItem::AppName, Instant::now()),
        Some("mail-agent".to_string())
    );
    assert_eq!(
        (chat.thread_name(), chat.last_terminal_title.clone()),
        (
            Some("Saved project title".to_string()),
            Some("mail-agent".to_string())
        )
    );

    let width = 80;
    let height = chat.desired_height(width);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| chat.render(frame.area(), frame.buffer_mut()))
        .unwrap();
    assert_chatwidget_snapshot!(
        "session_alias_footer",
        normalized_backend_snapshot(terminal.backend())
    );

    chat.session_alias.reader().update(None);
    chat.on_session_alias_updated(thread_id);
    assert_eq!(
        chat.terminal_title_value_for_item(TerminalTitleItem::AppName, Instant::now()),
        Some("codex".to_string())
    );
    assert_eq!(
        (chat.thread_name(), chat.last_terminal_title.clone()),
        (
            Some("Saved project title".to_string()),
            Some("Saved project title".to_string())
        )
    );
}

#[tokio::test]
async fn the_default_footer_shows_the_alias_without_overriding_custom_selections() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    chat.session_alias
        .reader()
        .update(Some("mail-agent".to_string()));
    chat.local_settings.tui.status_line = None;
    assert_eq!(
        chat.configured_status_line_items(),
        vec![
            "thread-name".to_string(),
            "model-with-reasoning".to_string(),
            "current-dir".to_string(),
        ]
    );
    chat.local_settings.tui.status_line = Some(vec!["model-name".to_string()]);
    assert_eq!(
        chat.configured_status_line_items(),
        vec!["model-name".to_string()]
    );
    chat.local_settings.tui.status_line = Some(vec![]);
    assert!(chat.configured_status_line_items().is_empty());

    chat.session_alias.reader().update(None);
    chat.local_settings.tui.status_line = None;
    assert_eq!(
        chat.configured_status_line_items(),
        DEFAULT_STATUS_LINE_ITEMS.map(ToString::to_string).to_vec()
    );
}

#[tokio::test]
async fn stale_alias_events_cannot_refresh_a_different_thread() {
    let (mut chat, _rx, _op_rx) = make_chatwidget_manual(/*model_override*/ None).await;
    let previous = ThreadId::new();
    let current = ThreadId::new();
    chat.thread_id = Some(current);
    chat.session_alias
        .set_thread(current, chat.app_event_tx.clone());
    chat.thread_name = Some("current-task".to_string());
    chat.local_settings.tui.terminal_title = Some(vec!["thread-name".to_string()]);
    chat.refresh_terminal_title();
    chat.session_alias
        .reader()
        .update(Some("current-agent".to_string()));

    chat.on_session_alias_updated(previous);
    assert_eq!(chat.last_terminal_title, Some("current-task".to_string()));
    chat.on_session_alias_updated(current);
    assert_eq!(chat.last_terminal_title, Some("current-agent".to_string()));
}
