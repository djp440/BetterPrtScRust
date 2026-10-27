//! 设置界面自动保存的时机：改完就存；正在输入文字或拖动数值时，停下约 500 ms 再存，
//! 不会每输一个字就写一次文件。不合法的设置不存，配置文件保持上一次的有效值。

use std::time::{Duration, Instant};

use bps_core::Settings;

/// 输入文字、拖动数值时，停下多久后保存。
pub(crate) const TYPING_PAUSE: Duration = Duration::from_millis(500);

/// 跟踪界面上的设置有没有待保存的改动。
pub(crate) struct AutoSave {
    /// 上一次看到的界面设置。
    seen: Settings,
    /// 有待保存的改动时，最晚在什么时候保存。
    due: Option<Instant>,
}

/// 这一帧该做什么。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// 没有要保存的改动（或改动不合法）。
    Idle,
    /// 还在输入，等这么久再看。
    Wait(Duration),
    Save,
}

impl AutoSave {
    pub(crate) fn new(saved: &Settings) -> Self {
        AutoSave {
            seen: saved.clone(),
            due: None,
        }
    }

    /// 每帧画完界面后调用。`settings` 是界面上的设置，`saved` 是配置文件里的设置；
    /// `busy` 表示正在输入文字或拖动数值。
    pub(crate) fn poll(
        &mut self,
        settings: &Settings,
        saved: &Settings,
        busy: bool,
        now: Instant,
    ) -> Decision {
        if *settings != self.seen {
            self.seen = settings.clone();
            self.due = Some(if busy { now + TYPING_PAUSE } else { now });
        }
        let Some(due) = self.due else {
            return Decision::Idle;
        };
        if busy && now < due {
            return Decision::Wait(due - now);
        }
        // 不管存不存，这次改动都处理完了；保存失败也不每帧重试，等下一次改动。
        self.due = None;
        if settings == saved || !settings.problems().is_empty() {
            Decision::Idle
        } else {
            Decision::Save
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bps_core::{DEFAULT_CONFIG, SettingsDocument};

    use super::*;

    fn saved() -> Settings {
        SettingsDocument::parse(DEFAULT_CONFIG)
            .unwrap()
            .settings()
            .clone()
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_click_is_saved_right_away() {
        let saved = saved();
        let mut auto = AutoSave::new(&saved);
        let now = Instant::now();
        assert_eq!(auto.poll(&saved, &saved, false, now), Decision::Idle);

        let mut settings = saved.clone();
        settings.autostart = !settings.autostart;
        assert_eq!(auto.poll(&settings, &saved, false, now), Decision::Save);
        // 只存一次。
        assert_eq!(
            auto.poll(&settings, &saved, false, now + ms(16)),
            Decision::Idle
        );
    }

    #[test]
    fn typing_is_saved_once_after_a_pause() {
        let saved = saved();
        let mut auto = AutoSave::new(&saved);
        let start = Instant::now();
        let mut settings = saved.clone();
        let mut dir = String::from(r"D:\");

        // 每 100 ms 输一个字，输入期间一次都不存。
        for (i, c) in "截图目录".chars().enumerate() {
            dir.push(c);
            settings.save_dir = Some(PathBuf::from(&dir));
            let now = start + ms(100 * i as u64);
            assert_eq!(
                auto.poll(&settings, &saved, true, now),
                Decision::Wait(TYPING_PAUSE),
                "第 {i} 个字"
            );
        }
        let last = start + ms(300);
        assert_eq!(
            auto.poll(&settings, &saved, true, last + ms(300)),
            Decision::Wait(ms(200))
        );
        assert_eq!(
            auto.poll(&settings, &saved, true, last + TYPING_PAUSE),
            Decision::Save
        );
        assert_eq!(
            auto.poll(&settings, &saved, true, last + ms(900)),
            Decision::Idle
        );
    }

    #[test]
    fn leaving_the_text_field_saves_without_waiting() {
        let saved = saved();
        let mut auto = AutoSave::new(&saved);
        let now = Instant::now();
        let mut settings = saved.clone();
        settings.filename_template.push('x');

        assert!(matches!(
            auto.poll(&settings, &saved, true, now),
            Decision::Wait(_)
        ));
        assert_eq!(
            auto.poll(&settings, &saved, false, now + ms(50)),
            Decision::Save
        );
    }

    #[test]
    fn invalid_settings_are_not_saved_until_fixed() {
        let saved = saved();
        let mut auto = AutoSave::new(&saved);
        let now = Instant::now();
        let mut settings = saved.clone();
        settings.save_dir = Some(PathBuf::new());
        assert!(!settings.problems().is_empty());

        assert_eq!(auto.poll(&settings, &saved, false, now), Decision::Idle);
        assert_eq!(
            auto.poll(&settings, &saved, false, now + ms(1000)),
            Decision::Idle
        );

        settings.save_dir = Some(PathBuf::from(r"D:\截图"));
        assert_eq!(
            auto.poll(&settings, &saved, false, now + ms(1100)),
            Decision::Save
        );
    }

    #[test]
    fn changing_a_value_back_writes_nothing() {
        let saved = saved();
        let mut auto = AutoSave::new(&saved);
        let now = Instant::now();
        let mut settings = saved.clone();
        settings.filename_template.push('x');
        auto.poll(&settings, &saved, true, now);
        settings.filename_template.pop();

        assert_eq!(
            auto.poll(&settings, &saved, false, now + ms(100)),
            Decision::Idle
        );
    }
}
