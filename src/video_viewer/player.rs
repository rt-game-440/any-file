//! 视频播放器：`MpvInstance`（vo=libmpv）+ 软件渲染上下文（ADR-0007 回退方案 C）。
//! 帧由 `mpv_render_context(sw)` 渲染为 RGBA 缓冲，再由 UI 上传为 `slint::Image`。

use std::ffi::{CString, c_int, c_void};
use std::path::Path;
use std::sync::Arc;

use crate::video::compositing::VideoFrameRgba;
use crate::video::error::VideoError;
use crate::video::ffi::{
    MPV_EVENT_NONE, MPV_RENDER_PARAM_API_TYPE, MPV_RENDER_PARAM_SW_FORMAT,
    MPV_RENDER_PARAM_SW_POINTER, MPV_RENDER_PARAM_SW_SIZE, MPV_RENDER_PARAM_SW_STRIDE,
    MPV_RENDER_UPDATE_FRAME, MpvRenderContext, MpvRenderParam,
};
use crate::video::loader::MpvLibrary;
use crate::video::mpv_backend::MpvInstance;
use crate::video::sandbox::validate_local_media_path;
use crate::video::transport::{MpvTransport, TransportSnapshot};

/// 字幕轨道信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrack {
    pub id: i64,
    pub title: String,
    pub lang: String,
    pub external: bool,
    pub selected: bool,
}

/// 视频播放器（仅限 UI 线程使用）
pub struct VideoPlayer {
    lib: Arc<MpvLibrary>,
    transport: MpvTransport,
    render_ctx: *mut MpvRenderContext,
    /// 是否有待取的新帧
    frame_pending: bool,
    subtitles_visible: bool,
}

impl VideoPlayer {
    pub fn new(lib: Arc<MpvLibrary>) -> Result<Self, VideoError> {
        let mpv = MpvInstance::new_sandboxed(
            Arc::clone(&lib),
            &[
                ("vo", "libmpv"),
                // 软件渲染上下文只能消费系统内存帧：优先硬解并回读（copy），失败自动回退软解
                ("hwdec", "auto-copy-safe"),
                ("sws-fast", "yes"),
                ("keep-open", "yes"),
                ("audio-display", "no"),
                ("sub-auto", "no"),
                ("pause", "yes"),
                ("idle", "yes"),
            ],
        )?;

        let api_type = CString::new("sw").expect("static cstr");
        let mut init_params = [
            MpvRenderParam {
                param_type: MPV_RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            MpvRenderParam::terminator(),
        ];
        let mut render_ctx: *mut MpvRenderContext = std::ptr::null_mut();
        // SAFETY: 句柄已初始化，参数数组以终止项结尾。
        let code = unsafe {
            (lib.mpv_render_context_create)(
                &mut render_ctx,
                mpv.raw_handle(),
                init_params.as_mut_ptr(),
            )
        };
        if code < 0 || render_ctx.is_null() {
            return Err(VideoError::MpvApiError {
                operation: "mpv_render_context_create(sw)",
                code,
                message: lib.error_string(code),
            });
        }
        Ok(Self {
            lib,
            transport: MpvTransport::new(mpv),
            render_ctx,
            frame_pending: false,
            subtitles_visible: true,
        })
    }

    pub fn transport(&self) -> &MpvTransport {
        &self.transport
    }

    /// 载入视频；`autoplay` 为真则立即开始播放。
    pub fn load(&mut self, path: &Path, autoplay: bool) -> Result<(), VideoError> {
        self.transport.set_paused(true);
        self.transport.load(path)?;
        self.frame_pending = true;
        if autoplay {
            self.transport.set_paused(false);
        }
        Ok(())
    }

    pub fn unload(&mut self) {
        self.transport.unload();
        self.frame_pending = false;
    }

    pub fn snapshot(&self) -> TransportSnapshot {
        self.transport.snapshot()
    }

    pub fn active_hwdec(&self) -> Option<String> {
        self.transport
            .mpv()
            .get_property_string("hwdec-current")
            .filter(|s| !s.is_empty() && s != "no")
    }

    /// 视频原始尺寸（宽, 高）。
    pub fn video_size(&self) -> Option<(u32, u32)> {
        let mpv = self.transport.mpv();
        let w = mpv.get_property_i64("width")?;
        let h = mpv.get_property_i64("height")?;
        (w > 0 && h > 0).then_some((w as u32, h as u32))
    }

    // ---------- 字幕 ----------

    pub fn subtitles_visible(&self) -> bool {
        self.subtitles_visible
    }

    pub fn set_subtitles_visible(&mut self, visible: bool) {
        self.subtitles_visible = visible;
        let _ = self
            .transport
            .mpv()
            .set_property_string("sub-visibility", if visible { "yes" } else { "no" });
    }

    pub fn toggle_subtitles(&mut self) {
        let v = !self.subtitles_visible;
        self.set_subtitles_visible(v);
    }

    /// 加载外挂字幕并选中。
    pub fn add_external_subtitle(&mut self, path: &Path) -> Result<(), VideoError> {
        let p = validate_local_media_path(path)?;
        self.transport.mpv().command(&["sub-add", &p, "select"])?;
        self.set_subtitles_visible(true);
        Ok(())
    }

    pub fn cycle_subtitle_track(&mut self) {
        let _ = self.transport.mpv().command(&["cycle", "sub"]);
    }

    pub fn subtitle_tracks(&self) -> Vec<SubtitleTrack> {
        let mpv = self.transport.mpv();
        let count = mpv.get_property_i64("track-list/count").unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..count {
            let get = |k: &str| mpv.get_property_string(&format!("track-list/{i}/{k}"));
            if get("type").as_deref() != Some("sub") {
                continue;
            }
            out.push(SubtitleTrack {
                id: mpv
                    .get_property_i64(&format!("track-list/{i}/id"))
                    .unwrap_or(0),
                title: get("title").unwrap_or_default(),
                lang: get("lang").unwrap_or_default(),
                external: get("external").as_deref() == Some("yes"),
                selected: get("selected").as_deref() == Some("yes"),
            });
        }
        out
    }

    // ---------- 渲染 ----------

    /// 是否有新帧待渲染（由 `mpv_render_context_update` 指示）。
    /// 标记需要按当前尺寸重绘（窗口缩放或暂停时无新帧的场景）。
    pub fn request_redraw(&mut self) {
        self.frame_pending = true;
    }

    pub fn has_new_frame(&mut self) -> bool {
        // 排空事件队列，避免 mpv 事件缓冲堆积
        while self.transport.mpv().wait_event(0.0).0 != MPV_EVENT_NONE {}
        // SAFETY: `render_ctx` 在 `self` 存活期间有效。
        let flags = unsafe { self.lib.render_context_update(self.render_ctx) };
        if flags & MPV_RENDER_UPDATE_FRAME != 0 {
            self.frame_pending = true;
        }
        self.frame_pending
    }

    /// 渲染当前帧到 `width x height` 的 RGBA 缓冲（保持比例，黑边由 mpv 绘制）。
    pub fn render_frame(&mut self, width: u32, height: u32) -> Result<VideoFrameRgba, VideoError> {
        let (w, h) = (width.max(2), height.max(2));
        let mut size: [c_int; 2] = [w as c_int, h as c_int];
        let fmt = CString::new("rgb0").expect("static cstr");
        let mut stride: usize = (w as usize) * 4;
        let mut rgba = vec![0u8; stride * h as usize];
        let mut params = [
            MpvRenderParam {
                param_type: MPV_RENDER_PARAM_SW_SIZE,
                data: size.as_mut_ptr().cast::<c_void>(),
            },
            MpvRenderParam {
                param_type: MPV_RENDER_PARAM_SW_FORMAT,
                data: fmt.as_ptr() as *mut c_void,
            },
            MpvRenderParam {
                param_type: MPV_RENDER_PARAM_SW_STRIDE,
                data: (&mut stride as *mut usize).cast::<c_void>(),
            },
            MpvRenderParam {
                param_type: MPV_RENDER_PARAM_SW_POINTER,
                data: rgba.as_mut_ptr().cast::<c_void>(),
            },
            MpvRenderParam::terminator(),
        ];
        // SAFETY: 缓冲区容量等于 stride*height，指针在调用期间有效。
        let code = unsafe {
            self.lib
                .render_context_render(self.render_ctx, params.as_mut_ptr())
        };
        if code < 0 {
            return Err(VideoError::MpvApiError {
                operation: "mpv_render_context_render(sw)",
                code,
                message: self.lib.error_string(code),
            });
        }
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        self.frame_pending = false;
        Ok(VideoFrameRgba {
            width: w,
            height: h,
            rgba,
        })
    }
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        if !self.render_ctx.is_null() {
            // libmpv 硬约束：必须先释放 render context，再销毁 mpv_handle（transport 随后 drop）
            // SAFETY: 上下文由 `mpv_render_context_create` 创建且仅释放一次。
            unsafe { self.lib.render_context_free(self.render_ctx) };
            self.render_ctx = std::ptr::null_mut();
        }
    }
}
