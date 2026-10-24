use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bps_platform::{HttpRequest, HttpResponse, HttpTransport, PlatformError};

#[derive(Default)]
pub(crate) struct HttpState {
    /// 依次返回的回复；用完后再请求会 panic（测试没有安排的请求就是 bug）。
    pub replies: VecDeque<Result<HttpResponse, String>>,
    /// 至今发出的每个请求。
    pub requests: Vec<HttpRequest>,
}

/// 假的 HTTP 传输：记下每个请求，按测试预先安排的顺序返回回复。
pub(crate) struct FakeHttpTransport(pub Arc<Mutex<HttpState>>);

impl HttpTransport for FakeHttpTransport {
    fn post(&self, request: &HttpRequest) -> Result<HttpResponse, PlatformError> {
        let mut state = self.0.lock().unwrap();
        state.requests.push(request.clone());
        match state
            .replies
            .pop_front()
            .expect("测试没有安排这个 HTTP 回复")
        {
            Ok(response) => Ok(response),
            Err(message) => Err(PlatformError::new(message)),
        }
    }
}
