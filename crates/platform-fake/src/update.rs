use std::sync::{Arc, Mutex};

use bps_platform::UpdateService;

#[derive(Default)]
pub(crate) struct UpdateState {
    /// 每次查询的仓库，按发起顺序排列；每一项都代表一次网络请求。
    pub requests: Vec<String>,
    /// 已发起、还没有返回结果的查询数。
    pub pending: usize,
}

/// 假的检查更新服务：只记下查询，不联网；结果由测试用
/// `FakePlatform::finish_update_check` 交回。
pub(crate) struct FakeUpdateService(pub Arc<Mutex<UpdateState>>);

impl UpdateService for FakeUpdateService {
    fn request_latest_release(&mut self, repository: &str) {
        let mut state = self.0.lock().unwrap();
        state.requests.push(repository.to_owned());
        state.pending += 1;
    }
}
