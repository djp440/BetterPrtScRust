use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bps_platform::{FileDialog, PlatformError, SaveAsOptions};

/// 假文件对话框：用户的回答由测试预先排好，弹出过的对话框都记下来供检查。
#[derive(Default)]
pub(crate) struct DialogState {
    /// 用户接下来几次在“另存为”对话框里的选择；`None` 表示点了取消。
    pub answers: VecDeque<Option<PathBuf>>,
    /// 至今弹出过的“另存为”对话框。
    pub shown: Vec<SaveAsOptions>,
}

pub(crate) struct FakeFileDialog(pub Arc<Mutex<DialogState>>);

impl FileDialog for FakeFileDialog {
    fn save_as(&mut self, options: &SaveAsOptions) -> Result<Option<PathBuf>, PlatformError> {
        let mut state = self.0.lock().unwrap();
        state.shown.push(options.clone());
        let answer = state
            .answers
            .pop_front()
            .expect("弹出了“另存为”对话框，但测试没有预先用 answer_save_as 安排用户的选择");
        Ok(answer)
    }
}
