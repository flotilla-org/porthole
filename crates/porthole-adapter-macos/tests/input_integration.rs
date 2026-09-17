#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use porthole_adapter_macos::MacOsAdapter;
use porthole_core::{
    adapter::{Adapter, ProcessLaunchSpec, RequireConfidence},
    input::{ClickButton, ClickSpec, KeyEvent},
    wait::WaitCondition,
};

fn spec_textedit() -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        app: "/System/Applications/TextEdit.app".to_string(),
        args: vec![],
        cwd: None,
        env: vec![],
        timeout: Duration::from_secs(10),
        require_confidence: RequireConfidence::Strong,
        require_fresh_surface: false,
        force_place: false,
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session, Accessibility, and Screen Recording permissions"]
async fn text_types_into_textedit_and_wait_dirty_fires() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    // Wait for the editor to be visible/stable first.
    adapter
        .wait(
            &surface,
            &WaitCondition::Stable {
                window_ms: 800,
                threshold_pct: 1.0,
            },
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .expect("initial stable");

    // Type text; expect the frame to go dirty.
    let baseline = adapter.screenshot(&surface).await.expect("baseline");
    adapter.text(&surface, "hello porthole\n").await.expect("text");
    let dirty = adapter
        .wait(
            &surface,
            &WaitCondition::Dirty { threshold_pct: 1.0 },
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .expect("dirty");
    assert_eq!(dirty.condition, "dirty");
    let _ = baseline;

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn key_event_triggers_dirty_after_typing() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    adapter.text(&surface, "x").await.expect("text");
    // Pressing Enter should cause a frame change.
    adapter
        .key(
            &surface,
            &[KeyEvent {
                key: "Enter".into(),
                modifiers: vec![],
            }],
        )
        .await
        .expect("key Enter");
    let dirty = adapter
        .wait(
            &surface,
            &WaitCondition::Dirty { threshold_pct: 1.0 },
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .expect("dirty");
    assert_eq!(dirty.condition, "dirty");

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn click_inside_window_is_accepted() {
    let adapter = MacOsAdapter::new();
    let outcome = adapter.launch_process(&spec_textedit()).await.expect("launch");
    let surface = outcome.surface;

    adapter
        .click(
            &surface,
            &ClickSpec {
                x: 100.0,
                y: 100.0,
                button: ClickButton::Left,
                count: 1,
                modifiers: vec![],
            },
        )
        .await
        .expect("click");

    adapter.close(&surface).await.expect("close");
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn attention_and_displays_return_non_empty() {
    let adapter = MacOsAdapter::new();
    let attention = adapter.attention().await.expect("attention");
    let displays = adapter.displays().await.expect("displays");
    assert!(!displays.is_empty(), "displays should list at least one monitor");
    // Cursor position should be inside some display bounds.
    let any_inside = displays.iter().any(|d| {
        attention.cursor.x >= d.bounds.x
            && attention.cursor.x < d.bounds.x + d.bounds.w
            && attention.cursor.y >= d.bounds.y
            && attention.cursor.y < d.bounds.y + d.bounds.h
    });
    assert!(any_inside, "cursor position should fall within some display");
}

/// Press identity on a real window: a key held with shift then released
/// through its press id, a left button held across a drag and released where
/// the drag ended, and `release_held` with nothing left over. The frame must
/// change after the typing; the drag selects the typed text.
#[tokio::test]
#[ignore = "requires a real macOS desktop session + permissions"]
async fn press_identity_primitives_drive_textedit() {
    use porthole_core::{
        adapter::ArtifactLaunchSpec,
        input::{ButtonSpec, KeyStrokeSpec, Modifier, PointerMoveSpec, PressAction},
    };
    let adapter = MacOsAdapter::new();
    // Open a document of our own rather than launching the app: a bare
    // launch may show the open panel, and reusing a running instance would
    // type into whatever document it has up.
    let path = std::env::temp_dir().join(format!("porthole-press-identity-{}.txt", std::process::id()));
    std::fs::write(&path, "press identity\n").expect("temp document");
    let outcome = adapter
        .launch_artifact(&ArtifactLaunchSpec {
            path: path.clone(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: true,
            force_place: false,
            timeout: Duration::from_secs(10),
        })
        .await
        .expect("open document");
    let surface = outcome.surface;
    adapter
        .wait(
            &surface,
            &WaitCondition::Stable {
                window_ms: 800,
                threshold_pct: 1.0,
            },
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .expect("initial stable");
    adapter.focus(&surface).await.expect("focus");

    // "A" through a held shift: the shift press is bound at its down; the
    // KeyA press carries the shift flag on its own events.
    let stroke = |press, action, key: &str, modifiers: Vec<Modifier>| KeyStrokeSpec {
        press,
        action,
        key: key.into(),
        modifiers,
    };
    adapter
        .key_stroke(&surface, &stroke(1, PressAction::Down, "ShiftLeft", vec![Modifier::Shift]))
        .await
        .expect("shift down");
    for press in 2..14u64 {
        adapter
            .key_stroke(&surface, &stroke(press, PressAction::Down, "KeyA", vec![Modifier::Shift]))
            .await
            .expect("a down");
        adapter
            .key_stroke(&surface, &stroke(press, PressAction::Up, "KeyA", vec![Modifier::Shift]))
            .await
            .expect("a up");
    }
    adapter
        .key_stroke(&surface, &stroke(1, PressAction::Up, "ShiftLeft", vec![]))
        .await
        .expect("shift up");
    let dirty = adapter
        .wait(
            &surface,
            // A dozen letters in a document window are well under a
            // percent of its pixels.
            &WaitCondition::Dirty { threshold_pct: 0.02 },
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .expect("dirty after typing");
    assert_eq!(dirty.condition, "dirty");

    // Drag across the text: down, motion while held (a drag), up at the end.
    adapter
        .button(
            &surface,
            &ButtonSpec {
                x: 40.0,
                y: 90.0,
                button: ClickButton::Left,
                action: PressAction::Down,
                modifiers: vec![],
            },
        )
        .await
        .expect("button down");
    for x in [60.0, 90.0, 120.0] {
        adapter.pointer_move(&surface, &PointerMoveSpec { x, y: 90.0 }).await.expect("drag");
    }
    adapter
        .button(
            &surface,
            &ButtonSpec {
                x: 120.0,
                y: 90.0,
                button: ClickButton::Left,
                action: PressAction::Up,
                modifiers: vec![],
            },
        )
        .await
        .expect("button up");
    adapter.release_held(&surface).await.expect("nothing held");

    // Held state left behind on purpose is released by release_held.
    adapter
        .key_stroke(&surface, &stroke(9, PressAction::Down, "KeyB", vec![]))
        .await
        .expect("b down");
    adapter.release_held(&surface).await.expect("release b");

    // The typed text made the document dirty; TextEdit vetoes the first
    // close with a save sheet, which Cmd-D ("Don't Save") dismisses.
    let _ = std::fs::remove_file(&path);
    if let Err(e) = adapter.close(&surface).await {
        assert_eq!(e.code, porthole_core::ErrorCode::CloseFailed, "{e:?}");
        adapter
            .key(
                &surface,
                &[KeyEvent {
                    key: "KeyD".into(),
                    modifiers: vec![Modifier::Cmd],
                }],
            )
            .await
            .expect("dismiss save sheet");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
