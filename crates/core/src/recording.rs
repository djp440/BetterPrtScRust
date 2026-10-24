//! 录屏：在实时画面上框选区域 → 录制 → 再按一次同一个热键停止 → 自动存储并把文件放进剪贴板。
//!
//! 和区域捕获一样，一次录屏跨越很多平台事件，所以它是常驻实例持有的一个进行中的会话：
//! [`Recording::start`] 打开实时画面上的选区遮罩后立即返回；遮罩上的输入交给
//! [`Recording::on_overlay_input`]，确定区域后开始录制；录制期间平台按帧率递交节拍
//! （[`Recording::on_tick`]）；再次收到同一个捕获请求时 [`Recording::stop`]。
//!
//! 时间轴（恒定帧率）：第 `k` 帧的时间戳是 `k / 帧率`，它显示的是到这一刻为止视频源给出的最新
//! 画面。每次节拍按单调时间算出到此刻应当已经写入多少帧，把缺的帧补上（画面没变时重复上一帧），
//! 所以节拍晚到、合并都不会改变帧率和时长。停止时补到停止那一刻。
//!
//! 产物先写进存储目录里的临时文件 `<文件名>.<扩展名>.part`，录完再改名成命名模板生成的文件名，
//! 录制失败或放弃时删除临时文件。
//!
//! 录制期间区域边框外显示录屏控制条（见 [`toolbar`]）：已录时长、暂停 / 继续、停止。
//! 暂停期间不计时、不写帧：时间轴按“已录时长”（单调时间减去暂停的时长）计算，继续之后
//! 接着上一帧的时间戳往下排，视频里没有暂停期间的画面。暂停期间仍按节拍读取视频源，
//! 继续时用的是此刻最新的画面。

mod toolbar;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use bps_platform::{
    Backdrop, ClipboardItem, IncomingRequest, NewFile, Notification, NotificationClick,
    OverlayInput, OverlayScene, Platform, PlatformError, Rect, StreamFrame, StreamOptions,
    VideoFormat, VideoFrame, VideoSettings, VideoStream, VideoTarget, VideoWriter,
};

use crate::capture::{self, CaptureFailure};
use crate::filename::FilenameFields;
use crate::output::DEFAULT_SAVE_SUBDIR;
use crate::selection::{RegionSelection, Step};
use crate::snap::Snapping;
use crate::stitch::virtual_desktop_bounds;
use crate::{CaptureMode, CaptureRequest, CoreError, RecordingFormat, SaveDir, Texts};
use toolbar::{Button, Controls, Toolbar};

/// MP4 的默认帧率。
const DEFAULT_MP4_FRAME_RATE: u32 = 30;
/// MP4 的最小宽高（像素）。Windows 上的 H.264 编码器（硬件和系统自带的软件编码器）都不接受
/// 宽或高不超过 32 像素的画面。
const MP4_MIN_SIZE: u32 = 34;
/// 同名文件最多尝试到 `-N` 的 N。
const MAX_NAME_SUFFIX: u32 = 9999;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// 一次录屏是怎么结束的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// 录屏已存储。
    Saved,
    /// 用户在开始录制前取消了，没有任何产物。
    Cancelled,
}

/// 进行中的录屏。
pub(crate) struct Recording {
    request: CaptureRequest,
    phase: Phase,
    /// 开始录屏时的界面语言：遮罩上的提示和录屏结束的通知都用它。
    texts: &'static Texts,
    /// 经 CLI 转交来、等录屏结束再回复的请求。
    pub requester: Option<IncomingRequest>,
}

enum Phase {
    /// 在实时画面上框选区域。
    Selecting {
        selection: Box<RegionSelection>,
        /// 遮罩上当前画着的场景；场景没变时不重画。
        drawn: Option<OverlayScene>,
    },
    /// 正在录制。
    Recording(Box<Active>),
    /// 已经结束（停止或放弃之后）。
    Ended,
}

/// 录制中的状态。
struct Active {
    stream: Box<dyn VideoStream>,
    writer: Box<dyn VideoWriter>,
    /// 视频正写入的临时文件。
    temp: PathBuf,
    /// 录完后的正式文件名（不含扩展名与 `-N` 后缀）所在的目录、主干与扩展名。
    dir: PathBuf,
    stem: String,
    extension: &'static str,
    frame_rate: u32,
    /// 开始录制时的单调时间。
    started: Duration,
    /// 已经结束的各次暂停的总时长。
    paused_total: Duration,
    /// 正暂停着时，暂停开始的单调时间。
    paused_at: Option<Duration>,
    /// 已经写入的帧数。
    written: u64,
    /// 视频源最近给出的一帧。
    latest: Option<Rc<dyn VideoFrame>>,
    /// 录屏控制条；读不到显示器布局时没有。
    controls: Option<Controls>,
}

/// 录屏的参数（热键绑定没有覆盖的项取默认值）。
struct Settings {
    format: VideoFormat,
    frame_rate: u32,
    show_cursor: bool,
}

impl Settings {
    fn of(request: &CaptureRequest) -> Result<Self, CoreError> {
        let options = &request.recording;
        let format = match options.format.unwrap_or(RecordingFormat::Mp4) {
            RecordingFormat::Mp4 => VideoFormat::Mp4,
            unsupported @ RecordingFormat::Gif => {
                return Err(CoreError::UnsupportedRecordingFormat(unsupported));
            }
        };
        Ok(Self {
            format,
            frame_rate: options.frame_rate.unwrap_or(DEFAULT_MP4_FRAME_RATE),
            show_cursor: options.show_cursor.unwrap_or(true),
        })
    }

    fn extension(&self) -> &'static str {
        match self.format {
            VideoFormat::Mp4 => "mp4",
        }
    }

    /// 实际录制的范围。H.264 按 2×2 像素块采样色度，MP4 的宽高必须是偶数：
    /// 奇数时去掉最右一列 / 最下一行；去掉之后仍要不小于 [`MP4_MIN_SIZE`]。
    fn video_rect(&self, region: Rect) -> Result<Rect, CoreError> {
        let (rect, minimum) = match self.format {
            VideoFormat::Mp4 => (
                Rect::new(region.x, region.y, region.width & !1, region.height & !1),
                MP4_MIN_SIZE,
            ),
        };
        if rect.width < minimum || rect.height < minimum {
            return Err(CoreError::RecordingRegionTooSmall { region, minimum });
        }
        Ok(rect)
    }
}

impl Recording {
    /// 开始一次录屏。区域模式先打开实时画面上的选区遮罩，等用户框选要录制的区域；
    /// 显示器（鼠标所在的那台）和虚拟桌面模式不用框选，直接开始录制。
    /// 遮罩上的提示和通知用 `texts` 的语言。
    pub fn start(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
    ) -> Result<Self, CoreError> {
        Settings::of(&request)?;
        let mut recording = Self {
            request,
            phase: Phase::Ended,
            texts,
            requester: None,
        };
        let frames = platform.frame_source.as_ref();
        let target = match recording.request.mode {
            CaptureMode::Region => None,
            CaptureMode::Display => Some(capture::display_under_cursor(frames)?.bounds),
            CaptureMode::VirtualDesktop => {
                Some(virtual_desktop_bounds(&frames.displays()?).ok_or(CoreError::NoDisplay)?)
            }
            mode @ CaptureMode::Window => return Err(CoreError::UnsupportedCaptureMode(mode)),
        };
        if let Some(rect) = target {
            let active = recording.begin(platform, rect)?;
            recording.phase = Phase::Recording(Box::new(active));
            return Ok(recording);
        }
        // 框选时选区吸附到窗口边缘（#11）。
        let snapping = Snapping::of_desktop(platform.window_source.as_ref());
        let displays = platform.overlay.open(Backdrop::Live)?;
        let Some(selection) = RegionSelection::new(displays, texts.get("overlay-record-hint"))
        else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        recording.phase = Phase::Selecting {
            selection: Box::new(selection.with_snapping(snapping)),
            drawn: None,
        };
        if let Err(e) = recording.redraw(platform) {
            platform.overlay.close();
            return Err(e.into());
        }
        Ok(recording)
    }

    /// 这次录屏的捕获请求（来自热键绑定）。
    pub fn request(&self) -> &CaptureRequest {
        &self.request
    }

    /// 已经开始录制（不再是框选阶段）。
    pub fn is_recording(&self) -> bool {
        matches!(self.phase, Phase::Recording(_))
    }

    /// 处理用户在遮罩（框选阶段）或录屏控制条（录制阶段）上的一次输入。返回 `None` 表示
    /// 录屏还在进行；否则这次录屏已经结束。
    pub fn on_overlay_input(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let selection = match &mut self.phase {
            Phase::Selecting { selection, .. } => selection,
            Phase::Recording(_) => return self.on_toolbar_input(platform, input),
            Phase::Ended => return None,
        };
        selection
            .snapping_mut()
            .load_controls(input, platform.window_source.as_ref());
        match selection.handle(input) {
            // 录屏的背景是实时画面，没有冻结帧可取色，也不显示放大镜：C 键不做事。
            Step::Continue | Step::CopyColor(_) => match self.redraw(platform) {
                Ok(()) => None,
                Err(e) => {
                    platform.overlay.close();
                    Some(Err(CaptureFailure::before_actions(e)))
                }
            },
            Step::Confirmed(region) => {
                platform.overlay.close();
                match self.begin(platform, region) {
                    Ok(active) => {
                        self.phase = Phase::Recording(Box::new(active));
                        None
                    }
                    Err(e) => Some(Err(CaptureFailure::before_actions(e))),
                }
            }
            Step::Cancelled => {
                platform.overlay.close();
                Some(Ok(Ending::Cancelled))
            }
        }
    }

    /// 录屏控制条上的输入：悬停高亮，点“暂停 / 继续”切换暂停，点“停止”停止并存储。
    fn on_toolbar_input(
        &mut self,
        platform: &mut Platform,
        input: &OverlayInput,
    ) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Recording(active) = &mut self.phase else {
            return None;
        };
        let clicked = active.controls.as_mut().and_then(|c| c.handle(input));
        match clicked {
            Some(Button::Stop) => return Some(self.stop(platform)),
            Some(Button::PauseResume) => match active.toggle_pause(platform.clock.monotonic()) {
                Ok(true) => {}
                Ok(false) => return Some(self.stop(platform)),
                Err(e) => return Some(Err(self.abandon(platform, e))),
            },
            None => {}
        }
        active.refresh_toolbar(platform);
        None
    }

    /// 节拍到了：把到此刻为止应有的帧写进视频。录制目标消失时停止并存储已录下的内容。
    pub fn on_tick(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Recording(active) = &mut self.phase else {
            return None;
        };
        let due = active.due(platform.clock.monotonic());
        match active.catch_up(due) {
            Ok(true) => {
                active.refresh_toolbar(platform);
                None
            }
            Ok(false) => Some(self.stop(platform)),
            Err(e) => Some(Err(self.abandon(platform, e))),
        }
    }

    /// 停止录制：补齐到此刻的帧，写完视频文件，按命名模板存储，并把文件放进剪贴板。
    /// 还在框选时关闭遮罩，相当于取消。
    pub fn stop(&mut self, platform: &mut Platform) -> Result<Ending, CaptureFailure> {
        match std::mem::replace(&mut self.phase, Phase::Ended) {
            Phase::Selecting { .. } => {
                platform.overlay.close();
                Ok(Ending::Cancelled)
            }
            Phase::Recording(active) => (*active).finish(platform, self.texts),
            Phase::Ended => Ok(Ending::Cancelled),
        }
    }

    /// 录制出错：放弃这段录屏，删除临时文件。
    fn abandon(&mut self, platform: &mut Platform, error: PlatformError) -> CaptureFailure {
        if let Phase::Recording(active) = std::mem::replace(&mut self.phase, Phase::Ended) {
            (*active).discard(platform);
        }
        CaptureFailure::before_actions(error)
    }

    /// 开始录制 `region`：建好临时文件，打开视频流和编码器，开始节拍。
    fn begin(&self, platform: &mut Platform, region: Rect) -> Result<Active, CoreError> {
        let settings = Settings::of(&self.request)?;
        let rect = settings.video_rect(region)?;
        let dir = save_dir(platform, &self.request)?;
        platform.file_system.create_dir_all(&dir)?;
        let stem = self
            .request
            .output
            .filename_template
            .render(&FilenameFields {
                captured_at: platform.clock.now(),
                mode: self.request.mode,
                window_title: None,
            });
        let extension = settings.extension();
        let temp = reserve_temp_file(platform, &dir, &stem, extension)?;
        let opened = (|| {
            let stream = platform.video_source.open(
                VideoTarget::Region(rect),
                StreamOptions {
                    show_cursor: settings.show_cursor,
                },
            )?;
            let writer = platform.video_encoder.start(
                &temp,
                &VideoSettings {
                    format: settings.format,
                    width: rect.width,
                    height: rect.height,
                    frame_rate: settings.frame_rate,
                },
            )?;
            platform
                .ticker
                .start(Duration::from_secs(1) / settings.frame_rate)?;
            Ok::<_, PlatformError>((stream, writer))
        })();
        let (stream, writer) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                let _ = platform.file_system.remove_file(&temp);
                return Err(e.into());
            }
        };
        let controls = platform
            .frame_source
            .displays()
            .ok()
            .map(|displays| Controls::new(Toolbar::beside(rect, &displays, self.texts)));
        let mut active = Active {
            stream,
            writer,
            temp,
            dir,
            stem,
            extension,
            frame_rate: settings.frame_rate,
            started: platform.clock.monotonic(),
            paused_total: Duration::ZERO,
            paused_at: None,
            written: 0,
            latest: None,
            controls,
        };
        active.refresh_toolbar(platform);
        Ok(active)
    }

    fn redraw(&mut self, platform: &mut Platform) -> Result<(), PlatformError> {
        let Phase::Selecting { selection, drawn } = &mut self.phase else {
            return Ok(());
        };
        let scene = selection.scene();
        if drawn.as_ref() != Some(&scene) {
            platform.overlay.render(&scene)?;
            *drawn = Some(scene);
        }
        Ok(())
    }
}

impl Active {
    /// 到单调时间 `now` 为止的已录时长：从开始录制算起，去掉暂停的时间。
    fn recorded(&self, now: Duration) -> Duration {
        let paused = self.paused_total + self.paused_at.map_or(Duration::ZERO, |at| now - at);
        now.saturating_sub(self.started).saturating_sub(paused)
    }

    /// 到单调时间 `now` 为止应当已经写入的帧数：时间戳早于此刻已录时长的帧都应写入。
    fn due(&self, now: Duration) -> u64 {
        let elapsed = self.recorded(now).as_nanos();
        (elapsed * u128::from(self.frame_rate)).div_ceil(NANOS_PER_SECOND) as u64
    }

    /// 暂停或继续。暂停前先把到此刻为止的帧写完。返回视频源是否还在。
    fn toggle_pause(&mut self, now: Duration) -> Result<bool, PlatformError> {
        match self.paused_at.take() {
            Some(at) => {
                self.paused_total += now.saturating_sub(at);
                Ok(true)
            }
            None => {
                let alive = self.catch_up(self.due(now))?;
                self.paused_at = Some(now);
                Ok(alive)
            }
        }
    }

    /// 按此刻的已录时长和暂停状态更新录屏控制条。
    fn refresh_toolbar(&mut self, platform: &mut Platform) {
        let recorded = self.recorded(platform.clock.monotonic());
        let paused = self.paused_at.is_some();
        if let Some(controls) = self.controls.as_mut() {
            controls.refresh(platform, recorded, paused);
        }
    }

    /// 第 `index` 帧的时间戳。
    fn timestamp(&self, index: u64) -> Duration {
        let nanos = u128::from(index) * NANOS_PER_SECOND / u128::from(self.frame_rate);
        Duration::from_nanos(nanos as u64)
    }

    /// 把视频补到 `due` 帧，缺的帧用视频源此刻最新的画面。返回视频源是否还在
    /// （`false` 表示录制目标已经消失）；还没有收到第一帧时先不写。
    ///
    /// 暂停期间不写帧，但照样读取视频源：继续时手里是此刻最新的画面，而不是视频源里
    /// 积压的、暂停期间的旧画面。
    fn catch_up(&mut self, due: u64) -> Result<bool, PlatformError> {
        if self.written >= due && self.paused_at.is_none() {
            return Ok(true);
        }
        let alive = match self.stream.latest()? {
            StreamFrame::Frame(frame) => {
                self.latest = Some(frame);
                true
            }
            StreamFrame::Pending => true,
            StreamFrame::Ended => false,
        };
        if let Some(frame) = self.latest.clone() {
            while self.written < due {
                self.writer
                    .write(frame.as_ref(), self.timestamp(self.written))?;
                self.written += 1;
            }
        }
        Ok(alive)
    }

    /// 停止录制：补齐到此刻的帧（至少一帧），写完视频，改名成命名模板生成的文件名，
    /// 发“录屏已保存”通知，再把文件放进剪贴板。
    fn finish(mut self, platform: &mut Platform, texts: &Texts) -> Result<Ending, CaptureFailure> {
        let due = self.due(platform.clock.monotonic()).max(1);
        let caught_up = self.catch_up(due);
        platform.ticker.stop();
        platform.recording_toolbar.hide();
        let Active {
            stream,
            writer,
            temp,
            dir,
            stem,
            extension,
            written,
            ..
        } = self;
        drop(stream);
        let finished = match caught_up {
            Err(e) => Err(CoreError::from(e)),
            Ok(_) if written == 0 => Err(CoreError::NothingRecorded),
            Ok(_) => writer.finish().map_err(CoreError::from),
        };
        if let Err(error) = finished {
            let _ = platform.file_system.remove_file(&temp);
            return Err(CaptureFailure::before_actions(error));
        }
        // 改名失败时保留临时文件：里面是录好的完整视频。
        let path = rename_to_final(platform, &temp, &dir, &stem, extension)
            .map_err(CaptureFailure::before_actions)?;
        platform.notifier.notify(Notification {
            title: texts.get("notification-recording-saved-title"),
            body: path.display().to_string(),
            on_click: Some(NotificationClick::OpenFile(path.clone())),
        });
        platform
            .clipboard
            .write(vec![ClipboardItem::Files(vec![path])])
            .map_err(CaptureFailure::before_actions)?;
        Ok(Ending::Saved)
    }

    /// 放弃这段录屏：停止节拍，关闭视频流和编码器，删除临时文件。
    fn discard(self, platform: &mut Platform) {
        platform.ticker.stop();
        platform.recording_toolbar.hide();
        let Active {
            stream,
            writer,
            temp,
            ..
        } = self;
        drop(stream);
        drop(writer);
        let _ = platform.file_system.remove_file(&temp);
    }
}

/// 把录好的临时文件改名为 `<stem>.<extension>`；同名文件已存在时依次尝试 `-1`、`-2`……
fn rename_to_final(
    platform: &mut Platform,
    temp: &std::path::Path,
    dir: &std::path::Path,
    stem: &str,
    extension: &str,
) -> Result<PathBuf, CoreError> {
    for suffix in 0..=MAX_NAME_SUFFIX {
        let path = dir.join(numbered(stem, suffix, extension));
        if platform.file_system.rename_new(temp, &path)? == NewFile::Created {
            return Ok(path);
        }
    }
    Err(CoreError::TooManySameNameFiles(
        dir.join(format!("{stem}.{extension}")),
    ))
}

/// 存储目录：热键绑定的 `save_dir`，默认是“图片”文件夹下的 `BetterPrtSc`。
fn save_dir(platform: &Platform, request: &CaptureRequest) -> Result<PathBuf, PlatformError> {
    Ok(match &request.output.save_dir {
        SaveDir::Default => platform
            .file_system
            .pictures_dir()?
            .join(DEFAULT_SAVE_SUBDIR),
        SaveDir::Custom(dir) => dir.clone(),
    })
}

/// 在 `dir` 里建一个空的临时文件 `<stem>.<extension>.part`（重名时加 `-N`），占住文件名。
fn reserve_temp_file(
    platform: &mut Platform,
    dir: &std::path::Path,
    stem: &str,
    extension: &str,
) -> Result<PathBuf, CoreError> {
    for suffix in 0..=MAX_NAME_SUFFIX {
        let path = dir.join(numbered(stem, suffix, &format!("{extension}.part")));
        if platform.file_system.create_new(&path, &[])? == NewFile::Created {
            return Ok(path);
        }
    }
    Err(CoreError::TooManySameNameFiles(
        dir.join(format!("{stem}.{extension}.part")),
    ))
}

/// `stem.extension`，`suffix` 不为 0 时是 `stem-suffix.extension`。
fn numbered(stem: &str, suffix: u32, extension: &str) -> String {
    match suffix {
        0 => format!("{stem}.{extension}"),
        n => format!("{stem}-{n}.{extension}"),
    }
}
