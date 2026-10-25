//! 设置界面录制热键：按键事件 → 热键（与配置里的热键文本一致）。

use bps_core::{HotkeyRecorder, RecordingOutcome};
use bps_platform::{Hotkey, Key, KeyInput, ModifierKey, Modifiers, RecordedKey};

fn modifier(m: ModifierKey) -> RecordedKey {
    RecordedKey::Modifier {
        modifier: m,
        right: false,
    }
}

fn feed(recorder: &mut HotkeyRecorder, inputs: &[KeyInput]) -> RecordingOutcome {
    let mut last = RecordingOutcome::Waiting(Modifiers::NONE);
    for input in inputs {
        last = recorder.feed(*input);
    }
    last
}

#[test]
fn modifiers_then_main_key_is_recorded_as_config_hotkey_text() {
    let mut recorder = HotkeyRecorder::new();
    let outcome = feed(
        &mut recorder,
        &[
            KeyInput::down(modifier(ModifierKey::Ctrl)),
            KeyInput::down(modifier(ModifierKey::Alt)),
            KeyInput::down(RecordedKey::Main(Key::F(11))),
        ],
    );
    let RecordingOutcome::Recorded(hotkey) = outcome else {
        panic!("应录到热键：{outcome:?}");
    };
    assert_eq!(hotkey.to_string(), "Ctrl+Alt+F11");
    assert_eq!("Ctrl+Alt+F11".parse::<Hotkey>().unwrap(), hotkey);
}

#[test]
fn win_combination_and_print_screen_are_supported() {
    let mut recorder = HotkeyRecorder::new();
    let outcome = feed(
        &mut recorder,
        &[
            KeyInput::down(modifier(ModifierKey::Win)),
            KeyInput::down(modifier(ModifierKey::Shift)),
            KeyInput::down(RecordedKey::Main(Key::Letter('O'))),
        ],
    );
    assert_eq!(
        outcome,
        RecordingOutcome::Recorded(Hotkey::new(
            Modifiers::WIN.with(Modifiers::SHIFT),
            Key::Letter('O')
        ))
    );
    let mut recorder = HotkeyRecorder::new();
    let outcome = recorder.feed(KeyInput::down(RecordedKey::Main(Key::PrintScreen)));
    assert_eq!(
        outcome,
        RecordingOutcome::Recorded(Hotkey::new(Modifiers::NONE, Key::PrintScreen))
    );
}

#[test]
fn print_screen_release_alone_still_records() {
    // 系统有时只递交 PrintScreen 的松开。
    let mut recorder = HotkeyRecorder::new();
    let outcome = recorder.feed(KeyInput::up(RecordedKey::Main(Key::PrintScreen)));
    assert_eq!(
        outcome,
        RecordingOutcome::Recorded(Hotkey::new(Modifiers::NONE, Key::PrintScreen))
    );
}

#[test]
fn live_preview_shows_held_modifiers_and_forgets_released_ones() {
    let mut recorder = HotkeyRecorder::new();
    assert_eq!(
        recorder.feed(KeyInput::down(modifier(ModifierKey::Shift))),
        RecordingOutcome::Waiting(Modifiers::SHIFT)
    );
    assert_eq!(
        recorder.feed(KeyInput::up(modifier(ModifierKey::Shift))),
        RecordingOutcome::Waiting(Modifiers::NONE)
    );
}

#[test]
fn left_and_right_modifier_are_counted_separately() {
    let mut recorder = HotkeyRecorder::new();
    let right = RecordedKey::Modifier {
        modifier: ModifierKey::Ctrl,
        right: true,
    };
    recorder.feed(KeyInput::down(modifier(ModifierKey::Ctrl)));
    recorder.feed(KeyInput::down(right));
    assert_eq!(
        recorder.feed(KeyInput::up(right)),
        RecordingOutcome::Waiting(Modifiers::CTRL)
    );
}

#[test]
fn escape_cancels_and_backspace_or_delete_clear() {
    for (key, expected) in [
        (RecordedKey::Escape, RecordingOutcome::Cancelled),
        (RecordedKey::Backspace, RecordingOutcome::Cleared),
        (RecordedKey::Delete, RecordingOutcome::Cleared),
    ] {
        let mut recorder = HotkeyRecorder::new();
        assert_eq!(recorder.feed(KeyInput::down(key)), expected);
    }
}

#[test]
fn unsupported_key_is_reported_and_recording_continues() {
    let mut recorder = HotkeyRecorder::new();
    assert_eq!(
        recorder.feed(KeyInput::down(RecordedKey::Other)),
        RecordingOutcome::Unsupported
    );
    assert_eq!(
        recorder.feed(KeyInput::down(RecordedKey::Main(Key::F(5)))),
        RecordingOutcome::Recorded(Hotkey::new(Modifiers::NONE, Key::F(5)))
    );
}

#[test]
fn after_finishing_input_is_ignored_and_release_of_all_keys_is_tracked() {
    let mut recorder = HotkeyRecorder::new();
    recorder.feed(KeyInput::down(modifier(ModifierKey::Win)));
    let done = recorder.feed(KeyInput::down(RecordedKey::Main(Key::F(1))));
    assert!(matches!(done, RecordingOutcome::Recorded(_)));
    assert!(!recorder.all_released());
    assert_eq!(
        recorder.feed(KeyInput::down(RecordedKey::Escape)),
        RecordingOutcome::Finished
    );
    recorder.feed(KeyInput::up(RecordedKey::Main(Key::F(1))));
    assert!(!recorder.all_released());
    recorder.feed(KeyInput::up(modifier(ModifierKey::Win)));
    assert!(recorder.all_released());
}
