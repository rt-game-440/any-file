//! 视频解码模块错误类型。

use std::fmt;
use std::path::PathBuf;

/// libmpv 动态加载、沙箱初始化与渲染过程中的结构化错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoError {
    /// 拒绝从相对路径加载 `mpv-2.dll`（防止 DLL 劫持）
    RelativeDllPathRejected(PathBuf),
    /// 未在可信目录找到 `mpv-2.dll`
    LibraryNotFound(PathBuf),
    /// 操作系统动态加载库失败
    LibraryLoadFailed { path: PathBuf, reason: String },
    /// 缺少必需的 libmpv 导出符号
    MissingSymbol { symbol: &'static str },
    /// libmpv ABI 版本不兼容（要求 2.x）
    IncompatibleAbiVersion { found: u64, required_major: u32 },
    /// 拒绝不安全的非本地媒体路径/网络协议
    UnsafeMediaPath(String),
    /// 创建 mpv 实例失败
    CreateFailed,
    /// 应用强制沙箱参数失败（严格中止，不降级放行）
    SandboxOptionFailed {
        option: String,
        value: String,
        code: i32,
        message: String,
    },
    /// mpv 实例初始化失败
    InitializeFailed { code: i32, message: String },
    /// mpv 命令或属性操作失败
    MpvApiError {
        operation: &'static str,
        code: i32,
        message: String,
    },
    /// 当前平台或渲染后端暂不支持所需操作
    Unsupported(String),
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeDllPathRejected(p) => {
                write!(f, "拒绝从相对路径加载 mpv-2.dll：{}", p.display())
            }
            Self::LibraryNotFound(p) => write!(f, "未在可信目录找到 mpv-2.dll：{}", p.display()),
            Self::LibraryLoadFailed { path, reason } => {
                write!(f, "加载 {} 失败：{reason}", path.display())
            }
            Self::MissingSymbol { symbol } => write!(f, "mpv-2.dll 缺少导出符号：{symbol}"),
            Self::IncompatibleAbiVersion {
                found,
                required_major,
            } => write!(
                f,
                "libmpv ABI 版本不兼容：实际 {found:#x}，要求主版本 >= {required_major}"
            ),
            Self::UnsafeMediaPath(reason) => write!(f, "拒绝不安全的媒体路径：{reason}"),
            Self::CreateFailed => write!(f, "mpv_create() 返回空指针"),
            Self::SandboxOptionFailed {
                option,
                value,
                code,
                message,
            } => write!(
                f,
                "应用强制沙箱参数 {option}={value} 失败（code={code}）：{message}"
            ),
            Self::InitializeFailed { code, message } => {
                write!(f, "mpv_initialize() 失败（code={code}）：{message}")
            }
            Self::MpvApiError {
                operation,
                code,
                message,
            } => write!(f, "libmpv 调用 {operation} 失败（code={code}）：{message}"),
            Self::Unsupported(msg) => write!(f, "不支持的视频操作：{msg}"),
        }
    }
}

impl std::error::Error for VideoError {}
