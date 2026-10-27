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
//! 录制期间区域边框外显示录屏控制条（见 [`toolbar`]）：已录时长、系统声音 / 麦克风开关、
//! 暂停 / 继续、停止。
//! 暂停期间不计时、不写帧：时间轴按“已录时长”（单调时间减去暂停的时长）计算，继续之后
//! 接着上一帧的时间戳往下排，视频里没有暂停期间的画面。暂停期间仍按节拍读取视频源，
//! 继续时用的是此刻最新的画面。
//!
//! 声音（见 [`AudioMix`]）：MP4 总带一条音轨，系统声音（默认开）和麦克风（默认关）混在里面；
//! 两路都关时音轨是静音，录屏控制条上的开关可以在录制中途打开、关掉任意一路（只影响这次录制）。
//! 每个节拍读取各路音频流，按每段声音的采集时间换算出它在时间轴上的位置（同样去掉暂停的时长），
//! 和画面用同一条时间轴：同一路声音之间的空白是静音，与这一路已有的部分明显重叠时丢掉重叠的部分；
//! 暂停期间采集的声音不录。各路都交到某一刻之后才把这一刻之前混好的声音写进音轨（有一路迟迟
//! 不交时最多等 [`MIX_LATENCY`]），停止时用静音把音轨补到与画面等长。
//! 打不开某一路（例如没有输出设备、没有麦克风）时这一路不录并通知用户；选中的麦克风不存在时
//! 改用系统默认麦克风并通知；编码器不支持音轨时照常录无声视频并通知；录制中途某一路出错时
//! 通知一次，之后这一路是静音。
//!
//! 编码与码率：MP4 按设置的编码（H.264 / H.265 / AV1）和画质（档位或自定义码率，见
//! [`Settings::bitrate`]）编码；编码器不支持选中的编码时改用 H.264 并通知用户。

mod toolbar;

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use bps_platform::{
    AudioChunk, AudioFormat, AudioStream, Backdrop, ClipboardItem, IncomingRequest, NewFile,
    Notification, NotificationClick, OverlayInput, OverlayScene, Platform, PlatformError, Rect,
    StreamFrame, StreamOptions, TopLevelWindow, VideoCodec, VideoFormat, VideoFrame, VideoSettings,
    VideoStream, VideoTarget, VideoWriter,
};

use crate::capture::{self, CaptureFailure};
use crate::filename::FilenameFields;
use crate::output::DEFAULT_SAVE_SUBDIR;
use crate::selection::{RegionSelection, Step};
use crate::snap::Snapping;
use crate::stitch::virtual_desktop_bounds;
use crate::window_selection::{WindowSelection, WindowStep};
use crate::{
    CaptureMode, CaptureRequest, CoreError, RecordingFormat, SaveDir, Texts, VideoQuality,
};
use toolbar::{AudioSwitches, Button, Controls, Toolbar};

/// MP4 的默认帧率。
const DEFAULT_MP4_FRAME_RATE: u32 = 30;
/// GIF 的默认帧率：每帧最多 256 色，文件比 MP4 大得多，所以默认取低一些。
const DEFAULT_GIF_FRAME_RATE: u32 = 15;
/// MP4 的最小宽高（像素）。Windows 上的 H.264 编码器（硬件和系统自带的软件编码器）都不接受
/// 宽或高不超过 32 像素的画面。
const MP4_MIN_SIZE: u32 = 34;
/// GIF 没有编码器的尺寸限制，只要求画面不是空的。
const GIF_MIN_SIZE: u32 = 1;
/// 同名文件最多尝试到 `-N` 的 N。
const MAX_NAME_SUFFIX: u32 = 9999;
const NANOS_PER_SECOND: u128 = 1_000_000_000;
/// 声音的位置与已写入的音轨相差不超过它时直接接上，不补静音也不裁剪：吸收采集时间的抖动。
const AUDIO_SYNC_TOLERANCE: Duration = Duration::from_millis(30);
/// 补静音时每次最多写入的时长，免得长时间的空白占用大量内存。
const SILENCE_BLOCK: Duration = Duration::from_secs(1);
/// 混音时最多等一路声音多久：某一路（例如系统没有播放声音时的回环）迟迟不交声音时，
/// 早于此刻这么久的部分照样写进音轨，这一路之后才交来的这段声音就不再混入。
const MIX_LATENCY: Duration = Duration::from_secs(1);
/// 各画质档位下 H.264 每像素每帧的码率（比特）：标准档 1080p 30 fps 约 7.5 Mbps。
const LOW_BITS_PER_PIXEL: f64 = 0.06;
const STANDARD_BITS_PER_PIXEL: f64 = 0.12;
const HIGH_BITS_PER_PIXEL: f64 = 0.24;
/// H.265、AV1 达到同样画质需要的码率约为 H.264 的这个比例。
const EFFICIENT_CODEC_RATIO: f64 = 0.6;
/// 按档位算出的码率的上下限（比特每秒）。
const MIN_BITRATE: f64 = 500_000.0;
const MAX_BITRATE: f64 = 100_000_000.0;

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
    /// 在实时画面上选择要录制的窗口。
    SelectingWindow {
        selection: Box<WindowSelection>,
        drawn: Option<OverlayScene>,
    },
    /// 正在录制。
    Recording(Box<Active>),
    /// 已经结束（停止或放弃之后）。
    Ended,
}

/// 录制的对象：确定下来的屏幕区域，或一个窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Region(Rect),
    Window(TopLevelWindow),
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
    /// 最近一次继续录制的单调时间（还没暂停过时是开始录制的时间）。
    resumed_at: Duration,
    /// 已经写入的帧数。
    written: u64,
    /// 视频源最近给出的一帧。
    latest: Option<Rc<dyn VideoFrame>>,
    /// 录屏控制条；读不到显示器布局时没有。
    controls: Option<Controls>,
    /// 音轨；GIF、编码器不支持音轨时没有。
    audio: Option<AudioMix>,
}

/// 混进音轨的一路声音。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioInput {
    System,
    Microphone,
}

impl AudioInput {
    const ALL: [AudioInput; 2] = [AudioInput::System, AudioInput::Microphone];
}

/// 录屏的音轨：把各路声音按采集时间排上时间轴、混成一路，排成从视频开头开始、首尾相接的样本。
struct AudioMix {
    format: AudioFormat,
    /// 按 [`AudioInput`] 的顺序：系统声音、麦克风。
    inputs: [MixInput; 2],
    /// 设置里选的麦克风；中途打开麦克风时也用它。
    microphone_device: Option<String>,
    /// 混好、还没写进音轨的样本，从第 `written` 个采样帧开始。
    buffer: Vec<f32>,
    /// 已经写进音轨的采样帧数。
    written: u64,
}

/// 混音里的一路声音。
#[derive(Default)]
struct MixInput {
    /// 音频流；这一路关着（或读取出错）时没有。
    stream: Option<Box<dyn AudioStream>>,
    /// 读取出错、还没通知用户的原因。
    lost: Option<PlatformError>,
    /// 这一路已经交到时间轴上的哪个采样帧。
    cursor: u64,
}

/// 录屏的参数（热键绑定和全局设置都没有设置的项取默认值）。
struct Settings {
    format: VideoFormat,
    frame_rate: u32,
    show_cursor: bool,
    codec: VideoCodec,
    quality: VideoQuality,
    /// 录入系统声音（只有 MP4）。
    system_audio: bool,
    /// 录入麦克风（只有 MP4）。
    microphone: bool,
    microphone_device: Option<String>,
}

impl Settings {
    fn of(request: &CaptureRequest) -> Result<Self, CoreError> {
        let options = &request.recording;
        let (format, default_frame_rate) = match options.format.unwrap_or(RecordingFormat::Mp4) {
            RecordingFormat::Mp4 => (VideoFormat::Mp4, DEFAULT_MP4_FRAME_RATE),
            RecordingFormat::Gif => (VideoFormat::Gif, DEFAULT_GIF_FRAME_RATE),
        };
        let mp4 = format == VideoFormat::Mp4;
        Ok(Self {
            format,
            frame_rate: options.frame_rate.unwrap_or(default_frame_rate),
            show_cursor: options.show_cursor.unwrap_or(true),
            codec: options.codec.unwrap_or_default(),
            quality: options.quality.unwrap_or_default(),
            system_audio: mp4 && options.system_audio.unwrap_or(true),
            microphone: mp4 && options.microphone.unwrap_or(false),
            microphone_device: options.microphone_device.clone(),
        })
    }

    /// 用 `codec` 编码 `rect` 大小的画面时的目标码率（比特每秒）：自定义码率原样使用；
    /// 档位按每像素每帧的比特数换算，H.265、AV1 打个折扣，再限制在合理范围内。
    fn bitrate(&self, rect: Rect, codec: VideoCodec) -> u32 {
        let bits_per_pixel = match self.quality {
            VideoQuality::Custom(kbps) => return kbps.saturating_mul(1000),
            VideoQuality::Low => LOW_BITS_PER_PIXEL,
            VideoQuality::Standard => STANDARD_BITS_PER_PIXEL,
            VideoQuality::High => HIGH_BITS_PER_PIXEL,
        };
        let ratio = match codec {
            VideoCodec::H264 => 1.0,
            VideoCodec::H265 | VideoCodec::Av1 => EFFICIENT_CODEC_RATIO,
        };
        let pixels = f64::from(rect.width) * f64::from(rect.height);
        (pixels * f64::from(self.frame_rate) * bits_per_pixel * ratio)
            .clamp(MIN_BITRATE, MAX_BITRATE) as u32
    }

    fn extension(&self) -> &'static str {
        match self.format {
            VideoFormat::Mp4 => "mp4",
            VideoFormat::Gif => "gif",
        }
    }

    /// 实际录制的范围。GIF 原样录制。H.264 按 2×2 像素块采样色度，MP4 的宽高必须是偶数：
    /// 奇数时去掉最右一列 / 最下一行；去掉之后仍要不小于 [`MP4_MIN_SIZE`]。
    fn video_rect(&self, region: Rect) -> Result<Rect, CoreError> {
        let (rect, minimum) = match self.format {
            VideoFormat::Mp4 => (
                Rect::new(region.x, region.y, region.width & !1, region.height & !1),
                MP4_MIN_SIZE,
            ),
            VideoFormat::Gif => (region, GIF_MIN_SIZE),
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
            CaptureMode::Window => return recording.start_window_selection(platform),
        };
        if let Some(rect) = target {
            let active = recording.begin(platform, Target::Region(rect))?;
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

    /// 以已经选好的 `target` 直接开始录制（等待确认界面上的“录屏”：不再框选）。
    /// 提示和通知用 `texts` 的语言。
    pub fn start_at(
        platform: &mut Platform,
        request: CaptureRequest,
        texts: &'static Texts,
        target: Target,
    ) -> Result<Self, CoreError> {
        Settings::of(&request)?;
        let mut recording = Self {
            request,
            phase: Phase::Ended,
            texts,
            requester: None,
        };
        let active = recording.begin(platform, target)?;
        recording.phase = Phase::Recording(Box::new(active));
        Ok(recording)
    }

    /// 窗口模式：在实时画面上高亮并选择要录制的顶层窗口。
    fn start_window_selection(mut self, platform: &mut Platform) -> Result<Self, CoreError> {
        let layout = platform.window_source.layout()?;
        let displays = platform.overlay.open(Backdrop::Live)?;
        let cursor = platform.frame_source.cursor_position().ok();
        let Some(selection) = WindowSelection::new(displays, layout, cursor) else {
            platform.overlay.close();
            return Err(CoreError::NoDisplay);
        };
        self.phase = Phase::SelectingWindow {
            selection: Box::new(selection),
            drawn: None,
        };
        if let Err(e) = self.redraw(platform) {
            platform.overlay.close();
            return Err(e.into());
        }
        Ok(self)
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
        if let Phase::SelectingWindow { selection, .. } = &mut self.phase {
            return match selection.handle(input) {
                WindowStep::Continue => self.redraw_or_fail(platform),
                WindowStep::Confirmed(window) => {
                    platform.overlay.close();
                    self.begin_recording(platform, Target::Window(window))
                }
                WindowStep::Cancelled => {
                    platform.overlay.close();
                    Some(Ok(Ending::Cancelled))
                }
            };
        }
        let selection = match &mut self.phase {
            Phase::Selecting { selection, .. } => selection,
            Phase::Recording(_) => return self.on_toolbar_input(platform, input),
            Phase::SelectingWindow { .. } | Phase::Ended => return None,
        };
        selection
            .snapping_mut()
            .load_controls(input, platform.window_source.as_ref());
        match selection.handle(input) {
            // 录屏的背景是实时画面，没有冻结帧可取色，也不显示放大镜：C 键不做事。
            Step::Continue | Step::CopyColor(_) => self.redraw_or_fail(platform),
            Step::Confirmed(region) => {
                platform.overlay.close();
                self.begin_recording(platform, Target::Region(region))
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
        let texts = self.texts;
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
            Some(Button::Audio(input)) => {
                if let Some(mix) = active.audio.as_mut() {
                    mix.toggle(platform, texts, input);
                }
            }
            None => {}
        }
        active.refresh_toolbar(platform);
        None
    }

    fn redraw_or_fail(
        &mut self,
        platform: &mut Platform,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match self.redraw(platform) {
            Ok(()) => None,
            Err(e) => {
                platform.overlay.close();
                Some(Err(CaptureFailure::before_actions(e)))
            }
        }
    }

    /// 选好对象后开始录制；失败时这次录屏结束。
    fn begin_recording(
        &mut self,
        platform: &mut Platform,
        target: Target,
    ) -> Option<Result<Ending, CaptureFailure>> {
        match self.begin(platform, target) {
            Ok(active) => {
                self.phase = Phase::Recording(Box::new(active));
                None
            }
            Err(e) => Some(Err(CaptureFailure::before_actions(e))),
        }
    }

    /// 节拍到了：把到此刻为止应有的帧写进视频。录制目标消失时停止并存储已录下的内容。
    pub fn on_tick(&mut self, platform: &mut Platform) -> Option<Result<Ending, CaptureFailure>> {
        let Phase::Recording(active) = &mut self.phase else {
            return None;
        };
        let now = platform.clock.monotonic();
        match active.catch_up(now, active.due(now)) {
            Ok(true) => {
                if let Some(mix) = active.audio.as_mut() {
                    mix.notify_lost(platform, self.texts);
                }
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
            Phase::Selecting { .. } | Phase::SelectingWindow { .. } => {
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

    /// 开始录制 `target`：建好临时文件，打开视频流和编码器，开始节拍。
    /// 窗口的画布尺寸在这一刻按窗口外框定下，之后不变。
    fn begin(&self, platform: &mut Platform, target: Target) -> Result<Active, CoreError> {
        let settings = Settings::of(&self.request)?;
        let (region, window) = match &target {
            Target::Region(region) => (*region, None),
            Target::Window(window) => (window.bounds, Some(window)),
        };
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
                window_title: window.map(|w| w.title.as_str()),
            });
        let extension = settings.extension();
        let temp = reserve_temp_file(platform, &dir, &stem, extension)?;
        let opened = (|| {
            let video_target = match window {
                Some(window) => VideoTarget::Window {
                    window: window.id,
                    width: rect.width,
                    height: rect.height,
                },
                None => VideoTarget::Region(rect),
            };
            let stream = platform.video_source.open(
                video_target,
                StreamOptions {
                    show_cursor: settings.show_cursor,
                },
            )?;
            let (writer, audio) = self.start_writer(platform, &temp, &settings, rect)?;
            platform
                .ticker
                .start(Duration::from_secs(1) / settings.frame_rate)?;
            Ok::<_, PlatformError>((stream, writer, audio))
        })();
        let (stream, writer, audio) = match opened {
            Ok(opened) => opened,
            Err(e) => {
                let _ = platform.file_system.remove_file(&temp);
                return Err(e.into());
            }
        };
        let audio = audio.map(|format| {
            let mut mix = AudioMix::new(format, settings.microphone_device.clone());
            for (input, on) in [
                (AudioInput::System, settings.system_audio),
                (AudioInput::Microphone, settings.microphone),
            ] {
                if on {
                    mix.open(platform, self.texts, input);
                }
            }
            mix
        });
        let controls = platform.frame_source.displays().ok().map(|displays| {
            Controls::new(Toolbar::beside(
                rect,
                &displays,
                self.texts,
                audio.is_some(),
            ))
        });
        let started = platform.clock.monotonic();
        let mut active = Active {
            stream,
            writer,
            temp,
            dir,
            stem,
            extension,
            frame_rate: settings.frame_rate,
            started,
            paused_total: Duration::ZERO,
            paused_at: None,
            resumed_at: started,
            written: 0,
            latest: None,
            controls,
            audio,
        };
        active.refresh_toolbar(platform);
        Ok(active)
    }

    /// 开始编码：MP4 带一条音轨（格式是音频源的格式）。编码器不支持选中的编码时改用 H.264，
    /// 不支持音轨时不带音轨，都通知用户；改到最后仍开不了时照常报告失败。
    /// 返回编码器和音轨的格式（没有音轨时为 `None`）。
    fn start_writer(
        &self,
        platform: &mut Platform,
        temp: &std::path::Path,
        settings: &Settings,
        rect: Rect,
    ) -> Result<(Box<dyn VideoWriter>, Option<AudioFormat>), PlatformError> {
        let mp4 = settings.format == VideoFormat::Mp4;
        let mut codec = settings.codec;
        let mut audio = mp4.then(|| platform.audio_source.format());
        let mut codec_error = None;
        let mut audio_error = None;
        let writer = loop {
            let video = VideoSettings {
                format: settings.format,
                width: rect.width,
                height: rect.height,
                frame_rate: settings.frame_rate,
                codec,
                bitrate: settings.bitrate(rect, codec),
                audio,
            };
            match platform.video_encoder.start(temp, &video) {
                Ok(writer) => break writer,
                Err(error) if mp4 && codec != VideoCodec::H264 => {
                    codec_error = Some((codec, error));
                    codec = VideoCodec::H264;
                }
                Err(error) if audio.is_some() => {
                    audio_error = Some(error);
                    audio = None;
                }
                Err(error) => return Err(error),
            }
        };
        if let Some((codec, error)) = codec_error {
            platform.notifier.notify(Notification {
                title: self
                    .texts
                    .get("notification-recording-codec-fallback-title"),
                body: self.texts.format(
                    "notification-recording-codec-fallback-body",
                    &[("codec", &codec.label()), ("reason", &error)],
                ),
                on_click: None,
            });
        }
        if let Some(error) = audio_error
            && (settings.system_audio || settings.microphone)
        {
            notify(
                platform,
                self.texts,
                "notification-recording-no-audio",
                &error,
            );
        }
        Ok((writer, audio))
    }

    fn redraw(&mut self, platform: &mut Platform) -> Result<(), PlatformError> {
        let (scene, drawn) = match &mut self.phase {
            Phase::Selecting { selection, drawn } => (selection.scene(), drawn),
            Phase::SelectingWindow { selection, drawn } => (selection.scene(), drawn),
            _ => return Ok(()),
        };
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
        let paused = self.paused_total
            + self
                .paused_at
                .map_or(Duration::ZERO, |at| now.saturating_sub(at));
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
                self.resumed_at = now;
                Ok(true)
            }
            None => {
                let alive = self.catch_up(now, self.due(now))?;
                self.paused_at = Some(now);
                Ok(alive)
            }
        }
    }

    /// 按此刻的已录时长、暂停状态和声音开关更新录屏控制条。
    fn refresh_toolbar(&mut self, platform: &mut Platform) {
        let recorded = self.recorded(platform.clock.monotonic());
        let paused = self.paused_at.is_some();
        let audio = self.audio.as_ref().map(|mix| AudioSwitches {
            system: mix.is_on(AudioInput::System),
            microphone: mix.is_on(AudioInput::Microphone),
        });
        if let Some(controls) = self.controls.as_mut() {
            controls.refresh(platform, recorded, paused, audio);
        }
    }

    /// 第 `index` 帧的时间戳。
    fn timestamp(&self, index: u64) -> Duration {
        let nanos = u128::from(index) * NANOS_PER_SECOND / u128::from(self.frame_rate);
        Duration::from_nanos(nanos as u64)
    }

    /// 单调时间 `now`：先把采集到的声音混进音轨，再把视频补到 `due` 帧，缺的帧用视频源此刻
    /// 最新的画面。返回视频源是否还在（`false` 表示录制目标已经消失）；还没有收到第一帧时先不写。
    ///
    /// 暂停期间不写帧，但照样读取视频源：继续时手里是此刻最新的画面，而不是视频源里
    /// 积压的、暂停期间的旧画面。
    fn catch_up(&mut self, now: Duration, due: u64) -> Result<bool, PlatformError> {
        self.pull_audio(now)?;
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

    /// 读取各路采集到的声音、混进音轨，把已经混好的部分写进视频（单调时间 `now`）。
    /// 暂停开始之后、继续之前采集的部分丢掉。
    fn pull_audio(&mut self, now: Duration) -> Result<(), PlatformError> {
        let Some(mut mix) = self.audio.take() else {
            return Ok(());
        };
        for input in AudioInput::ALL {
            for chunk in mix.read(input) {
                let start = chunk.captured_at.max(self.resumed_at);
                mix.place(input, &chunk, start, self.paused_at, self.recorded(start));
            }
        }
        let recorded = self.recorded(now);
        let flushed = mix.flush(self.writer.as_mut(), recorded);
        self.audio = Some(mix);
        flushed
    }

    /// 停止录制：补齐到此刻的帧（至少一帧），写完视频，改名成命名模板生成的文件名，
    /// 发“录屏已保存”通知，再把文件放进剪贴板。
    fn finish(mut self, platform: &mut Platform, texts: &Texts) -> Result<Ending, CaptureFailure> {
        let now = platform.clock.monotonic();
        let due = self.due(now).max(1);
        let recorded = self.recorded(now);
        let caught_up = self.catch_up(now, due).and_then(|alive| {
            if let Some(mix) = self.audio.as_mut() {
                mix.finish(self.writer.as_mut(), recorded)?;
            }
            Ok(alive)
        });
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

impl AudioMix {
    fn new(format: AudioFormat, microphone_device: Option<String>) -> Self {
        Self {
            format,
            inputs: Default::default(),
            microphone_device,
            buffer: Vec::new(),
            written: 0,
        }
    }

    fn input(&mut self, input: AudioInput) -> &mut MixInput {
        &mut self.inputs[input as usize]
    }

    /// 这一路正在录。
    fn is_on(&self, input: AudioInput) -> bool {
        self.inputs[input as usize].stream.is_some()
    }

    /// 时长 `duration` 含多少个采样帧（向下取整）。
    fn frames_in(&self, duration: Duration) -> u64 {
        (duration.as_nanos() * u128::from(self.format.sample_rate) / NANOS_PER_SECOND) as u64
    }

    fn channels(&self) -> usize {
        usize::from(self.format.channels)
    }

    /// 打开一路声音；打不开时通知用户，这一路不录。选中的麦克风打不开时改用系统默认麦克风。
    fn open(&mut self, platform: &mut Platform, texts: &Texts, input: AudioInput) {
        let opened = match input {
            AudioInput::System => platform
                .audio_source
                .open_system_audio()
                .map_err(|e| (e, "notification-recording-no-audio")),
            AudioInput::Microphone => {
                let device = self.microphone_device.as_deref();
                match platform.audio_source.open_microphone(device) {
                    Err(error) if device.is_some() => {
                        match platform.audio_source.open_microphone(None) {
                            Ok(stream) => {
                                notify(
                                    platform,
                                    texts,
                                    "notification-recording-microphone-fallback",
                                    &error,
                                );
                                Ok(stream)
                            }
                            Err(error) => Err(error),
                        }
                    }
                    opened => opened,
                }
                .map_err(|e| (e, "notification-recording-no-microphone"))
            }
        };
        match opened {
            Ok(stream) => {
                let written = self.written;
                let track = self.input(input);
                track.stream = Some(stream);
                // 已经写进音轨的部分来不及混入了。
                track.cursor = track.cursor.max(written);
            }
            Err((error, key)) => notify(platform, texts, key, &error),
        }
    }

    /// 录屏控制条上的开关：关掉正在录的一路（停止采集），或打开关着的一路。
    fn toggle(&mut self, platform: &mut Platform, texts: &Texts, input: AudioInput) {
        if self.is_on(input) {
            self.input(input).stream = None;
        } else {
            self.open(platform, texts, input);
        }
    }

    /// 取出一路上次读取以来采集到的声音；读取出错时关掉这一路，等节拍里通知用户。
    fn read(&mut self, input: AudioInput) -> Vec<AudioChunk> {
        let track = self.input(input);
        let Some(stream) = track.stream.as_mut() else {
            return Vec::new();
        };
        match stream.read() {
            Ok(chunks) => chunks,
            Err(error) => {
                track.stream = None;
                track.lost = Some(error);
                Vec::new()
            }
        }
    }

    /// 通知用户中途出错的各路声音（每次出错只通知一次）。
    fn notify_lost(&mut self, platform: &mut Platform, texts: &Texts) {
        for input in AudioInput::ALL {
            if let Some(error) = self.input(input).lost.take() {
                let key = match input {
                    AudioInput::System => "notification-recording-audio-lost",
                    AudioInput::Microphone => "notification-recording-microphone-lost",
                };
                notify(platform, texts, key, &error);
            }
        }
    }

    /// 把 `chunk` 里从单调时间 `start` 起（`end` 有值时到 `end` 为止）的声音混进 `input` 这一路，
    /// 它的开头在时间轴上的位置是 `position`。
    fn place(
        &mut self,
        input: AudioInput,
        chunk: &AudioChunk,
        start: Duration,
        end: Option<Duration>,
        position: Duration,
    ) {
        let channels = self.channels();
        let total = chunk.samples.len() / channels;
        let first = (self.frames_in(start.saturating_sub(chunk.captured_at)) as usize).min(total);
        let last = end.map_or(total, |end| {
            (self.frames_in(end.saturating_sub(chunk.captured_at)) as usize).min(total)
        });
        if first >= last {
            return;
        }
        let mut samples = &chunk.samples[first * channels..last * channels];
        let target = self.frames_in(position);
        let tolerance = self.frames_in(AUDIO_SYNC_TOLERANCE);
        let written = self.written;
        let track = &mut self.inputs[input as usize];
        if target > track.cursor + tolerance {
            track.cursor = target;
        } else if target + tolerance < track.cursor {
            let overlap = ((track.cursor - target) as usize * channels).min(samples.len());
            samples = &samples[overlap..];
        }
        // 已经写进音轨的部分来不及混入了。
        if track.cursor < written {
            let late = ((written - track.cursor) as usize * channels).min(samples.len());
            samples = &samples[late..];
            track.cursor += (late / channels) as u64;
        }
        let offset = (track.cursor - written) as usize * channels;
        track.cursor += (samples.len() / channels) as u64;
        let end = offset + samples.len();
        if self.buffer.len() < end {
            self.buffer.resize(end, 0.0);
        }
        for (mixed, sample) in self.buffer[offset..end].iter_mut().zip(samples) {
            *mixed += sample;
        }
    }

    /// 把各路都已交到的部分（以及早于时间轴上 `now` 超过 [`MIX_LATENCY`] 的部分）写进音轨。
    fn flush(&mut self, writer: &mut dyn VideoWriter, now: Duration) -> Result<(), PlatformError> {
        let buffered = self.written + (self.buffer.len() / self.channels()) as u64;
        let settled = self
            .inputs
            .iter()
            .filter(|track| track.stream.is_some())
            .map(|track| track.cursor)
            .min()
            .unwrap_or(buffered);
        let overdue = self.frames_in(now.saturating_sub(MIX_LATENCY));
        self.write_until(writer, settled.max(overdue))
    }

    /// 停止录制：把音轨写到时间轴上的 `end`（之后的声音不要，之前的空白补静音）。
    fn finish(&mut self, writer: &mut dyn VideoWriter, end: Duration) -> Result<(), PlatformError> {
        let end = self.frames_in(end);
        self.write_until(writer, end)?;
        self.buffer.clear();
        Ok(())
    }

    /// 把音轨写到第 `end` 个采样帧：先写混好的样本，不够的部分是静音。
    fn write_until(&mut self, writer: &mut dyn VideoWriter, end: u64) -> Result<(), PlatformError> {
        if end <= self.written {
            return Ok(());
        }
        let channels = self.channels();
        let from_buffer = ((end - self.written) as usize).min(self.buffer.len() / channels);
        if from_buffer > 0 {
            writer.write_audio(&self.buffer[..from_buffer * channels])?;
            self.buffer.drain(..from_buffer * channels);
            self.written += from_buffer as u64;
        }
        let block = self.frames_in(SILENCE_BLOCK).max(1);
        while self.written < end {
            let n = (end - self.written).min(block);
            writer.write_audio(&vec![0.0; n as usize * channels])?;
            self.written += n;
        }
        Ok(())
    }
}

/// 发一条关于录屏声音的通知：标题和正文是 `key` 加上 `-title`、`-body`，正文带上原因。
fn notify(platform: &mut Platform, texts: &Texts, key: &str, error: &PlatformError) {
    platform.notifier.notify(Notification {
        title: texts.get(&format!("{key}-title")),
        body: texts.format(&format!("{key}-body"), &[("reason", error)]),
        on_click: None,
    });
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
