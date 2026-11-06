use super::*;
use pretty_assertions::assert_eq;

#[test]
fn aliases_are_bounded_safe_display_text() {
    let unicode = "\u{63a7}\u{5236}\u{9762}";
    for (input, expected) in [
        ("  mail-agent \n", Some("mail-agent".to_string())),
        ("\n\n", None),
        ("\x1b\x07\u{202e}\u{200b}", None),
        ("mail\u{202e}-agent\x07", Some("mail-agent".to_string())),
        (unicode, Some(unicode.to_string())),
    ] {
        assert_eq!(parse_alias(input.as_bytes()).unwrap(), expected);
    }
    assert_eq!(
        parse_alias("x".repeat(MAX_ALIAS_CHARS + 1).as_bytes()).unwrap(),
        Some("x".repeat(MAX_ALIAS_CHARS))
    );
    assert!(parse_alias(b"first\nsecond\n").is_err());
    assert!(parse_alias(&[0xff]).is_err());
}

#[test]
fn only_alias_changes_invalidate_cached_transcript_layouts() {
    let alias = SessionAliasName::default();
    assert_eq!((alias.get(), alias.revision()), (None, None));
    assert!(alias.update(Some("mail-agent".to_string())));
    assert!(!alias.update(Some("mail-agent".to_string())));
    assert_eq!(
        (alias.get(), alias.revision()),
        (Some("mail-agent".to_string()), Some(1))
    );
    assert!(alias.update(None));
    assert_eq!((alias.get(), alias.revision()), (None, Some(2)));
}

#[test]
fn session_header_follows_the_shared_alias_and_restores_the_brand_when_cleared() {
    use crate::history_cell::HistoryCell;
    use crate::history_cell::SessionHeaderHistoryCell;

    let alias = SessionAliasName::default();
    let header = SessionHeaderHistoryCell::new(
        "gpt-5.6-sol".to_string(),
        /*reasoning_effort*/ None,
        PathBuf::from("project"),
        "test",
    )
    .with_session_alias(alias.clone());
    let mut rendered = Vec::new();
    for name in [None, Some("mail-agent"), Some("renamed-agent"), None] {
        alias.update(name.map(str::to_string));
        rendered.push(
            header
                .display_lines(/*width*/ 80)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        );
        assert_eq!(
            header.raw_lines().first().unwrap().to_string(),
            format!("{} (vtest)", name.unwrap_or("OpenAI Codex"))
        );
    }
    insta::assert_snapshot!(rendered.join("\n---\n"));

    alias.update(Some("\u{63a7}\u{5236}\u{9762}".repeat(20)));
    assert!(
        header
            .display_lines(/*width*/ 20)
            .iter()
            .all(|line| { crate::width::display_width(&line.to_string()) <= 20 })
    );
}

#[tokio::test]
async fn switching_threads_keeps_old_history_names_and_resets_the_live_name() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let tx = AppEventSender::new(tx);
    let first = ThreadId::new();
    let mut alias = SessionAlias::new(None);
    alias.set_thread(first, tx.clone());
    alias.name.update(Some("first-agent".to_string()));
    let old_reader = alias.reader();
    alias.set_thread(first, tx.clone());
    assert_eq!(alias.name(), Some("first-agent".to_string()));

    alias.set_thread(ThreadId::new(), tx);
    assert_eq!(
        (old_reader.get(), alias.name()),
        (Some("first-agent".to_string()), None)
    );
}

#[cfg(unix)]
fn fake_command(directory: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let command = directory.join("session-alias");
    std::fs::write(&command, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
    command
}

#[cfg(unix)]
#[tokio::test]
async fn native_queries_use_the_exact_thread_id_and_reject_bad_results() {
    let directory = tempfile::tempdir().unwrap();
    let thread_id = ThreadId::new();
    let command = fake_command(
        directory.path(),
        &format!(
            "[ \"$#\" = 2 ] && [ \"$1\" = name ] && [ \"$2\" = {thread_id} ] || exit 3\nprintf '%s\\n' mail-agent"
        ),
    );
    assert_eq!(
        lookup_alias(&command, thread_id).await.unwrap(),
        Some("mail-agent".to_string())
    );
    fake_command(directory.path(), "printf 'wrong-name'; exit 1");
    assert!(lookup_alias(&command, thread_id).await.is_err());
    fake_command(directory.path(), "head -c 4097 /dev/zero");
    assert_eq!(
        lookup_alias(&command, thread_id).await.unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    fake_command(directory.path(), "sleep 30");
    assert_eq!(
        lookup_alias(&command, thread_id).await.unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
}

#[cfg(unix)]
#[tokio::test]
async fn watcher_refreshes_renames_and_clears_without_persisting_codex_titles() {
    let directory = tempfile::tempdir().unwrap();
    let value_path = directory.path().join("name");
    std::fs::write(&value_path, "first-agent\n").unwrap();
    let command = fake_command(directory.path(), "cat \"$(dirname \"$0\")/name\"");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let thread_id = ThreadId::new();
    let mut alias = SessionAlias::new(Some(command));
    alias.set_thread(thread_id, AppEventSender::new(tx));
    for expected in [Some("first-agent"), Some("second-agent"), None] {
        std::fs::write(&value_path, expected.unwrap_or_default()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event, AppEvent::SessionAliasUpdated { thread_id: id } if id == thread_id)
        );
        assert_eq!(alias.name().as_deref(), expected);
    }
    let task = alias.task.as_ref().unwrap().abort_handle();
    drop(alias);
    tokio::time::timeout(Duration::from_secs(1), async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the widget stops its alias watcher");
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_and_timed_out_queries_stop_the_cli_process_group() {
    let directory = tempfile::tempdir().unwrap();
    let command = fake_command(
        directory.path(),
        "directory=$(dirname \"$0\")\ntouch \"$directory/started\"\n(sleep 2; touch \"$directory/survived\") &\nwait",
    );
    let started = directory.path().join("started");
    let survived = directory.path().join("survived");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut alias = SessionAlias::new(Some(command.clone()));
    alias.set_thread(ThreadId::new(), AppEventSender::new(tx));
    tokio::time::timeout(Duration::from_secs(3), async {
        while !started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the alias query starts");
    drop(alias);
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(!survived.exists(), "cancellation stops the CLI's children");

    assert_eq!(
        lookup_alias(&command, ThreadId::new())
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!survived.exists(), "timeout stops the CLI's children");
}
