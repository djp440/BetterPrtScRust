//! 录屏：Windows.Graphics.Capture 视频源 + Media Foundation MP4 编码器（占位，稍后实现）。

use std::path::Path;

use bps_platform::{
    PlatformError, StreamOptions, VideoEncoder, VideoSettings, VideoSource, VideoStream,
    VideoTarget, VideoWriter,
};

/// 录屏的视频源。
#[derive(Debug, Default)]
pub struct WgcVideoSource;

impl WgcVideoSource {
    pub fn new() -> Self {
        Self
    }
}

impl VideoSource for WgcVideoSource {
    fn open(
        &mut self,
        _target: VideoTarget,
        _options: StreamOptions,
    ) -> Result<Box<dyn VideoStream>, PlatformError> {
        Err(PlatformError::new("录屏尚未实现"))
    }
}

/// MP4 编码器。
#[derive(Debug, Default)]
pub struct MfVideoEncoder;

impl MfVideoEncoder {
    pub fn new() -> Self {
        Self
    }
}

impl VideoEncoder for MfVideoEncoder {
    fn start(
        &mut self,
        _path: &Path,
        _settings: &VideoSettings,
    ) -> Result<Box<dyn VideoWriter>, PlatformError> {
        Err(PlatformError::new("录屏尚未实现"))
    }
}
