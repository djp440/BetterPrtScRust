use std::sync::{Arc, Mutex};

use bps_platform::{TrayMenu, TrayMenuItem};

/// 记下 `core` 设置的托盘菜单，供测试查看文字、模拟选中。
pub(crate) struct FakeTrayMenu(pub Arc<Mutex<Vec<TrayMenuItem>>>);

impl TrayMenu for FakeTrayMenu {
    fn set_items(&mut self, items: Vec<TrayMenuItem>) {
        *self.0.lock().unwrap() = items;
    }
}
