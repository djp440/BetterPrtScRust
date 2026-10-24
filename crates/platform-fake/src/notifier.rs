use std::sync::{Arc, Mutex};

use bps_platform::{Notification, Notifier};

/// 把发出的通知按顺序记下来，供测试查询。
pub(crate) struct FakeNotifier(pub Arc<Mutex<Vec<Notification>>>);

impl Notifier for FakeNotifier {
    fn notify(&self, notification: Notification) {
        self.0.lock().unwrap().push(notification);
    }
}
