//! 手动检查 GitHub Release 新版本：无后台常驻、无遥测，仅在用户点击时联网一次。
//! 网络来源通过 `ReleaseSource` 抽象注入，测试使用本地桩。

use std::process::Command;

pub const RELEASE_API_URL: &str =
    "https://api.github.com/repos/rt-game-440/any-file/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/rt-game-440/any-file/releases";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    Available { version: String, url: String },
    Failed(String),
}

/// 最新发布信息来源（返回 GitHub `releases/latest` 的 JSON 文本）
pub trait ReleaseSource: Send + Sync {
    fn fetch_latest(&self) -> Result<String, String>;
}

/// 通过系统自带 `curl.exe` 拉取（避免引入 TLS 依赖）
pub struct CurlReleaseSource;

impl ReleaseSource for CurlReleaseSource {
    fn fetch_latest(&self) -> Result<String, String> {
        let mut cmd = Command::new("curl.exe");
        cmd.args([
            "-sSL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: AnyFile-update-check",
            RELEASE_API_URL,
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = cmd.output().map_err(|e| format!("无法调用 curl：{e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        String::from_utf8(out.stdout).map_err(|e| e.to_string())
    }
}

fn parse_version(s: &str) -> Option<Vec<u64>> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let parts: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// `latest` 是否严格新于 `current`
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => false,
    }
}

pub fn check_for_update(source: &dyn ReleaseSource, current: &str) -> UpdateStatus {
    let body = match source.fetch_latest() {
        Ok(b) => b,
        Err(e) => return UpdateStatus::Failed(e),
    };
    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return UpdateStatus::Failed(format!("响应解析失败：{e}")),
    };
    let Some(tag) = v.get("tag_name").and_then(|t| t.as_str()) else {
        return UpdateStatus::Failed("响应缺少 tag_name".to_string());
    };
    if !is_newer(tag, current) {
        return UpdateStatus::UpToDate;
    }
    let url = v
        .get("html_url")
        .and_then(|u| u.as_str())
        .filter(|u| crate::ports::is_allowed_url(u))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    UpdateStatus::Available {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(Result<String, String>);
    impl ReleaseSource for Stub {
        fn fetch_latest(&self) -> Result<String, String> {
            self.0.clone()
        }
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn available_up_to_date_and_failures() {
        let good = "https://github.com/rt-game-440/any-file/releases/tag/v0.2.0";
        let new = Stub(Ok(format!(r#"{{"tag_name":"v0.2.0","html_url":"{good}"}}"#)));
        assert_eq!(
            check_for_update(&new, "0.1.0"),
            UpdateStatus::Available {
                version: "0.2.0".into(),
                url: good.into()
            }
        );
        assert_eq!(check_for_update(&new, "0.2.0"), UpdateStatus::UpToDate);
        let evil = Stub(Ok(
            r#"{"tag_name":"v9.0.0","html_url":"https://evil.com/x"}"#.into()
        ));
        assert_eq!(
            check_for_update(&evil, "0.1.0"),
            UpdateStatus::Available {
                version: "9.0.0".into(),
                url: RELEASES_PAGE.into()
            }
        );
        for bad in [Err("offline".to_string()), Ok("not json".into()), Ok("{}".into())] {
            assert!(matches!(
                check_for_update(&Stub(bad), "0.1.0"),
                UpdateStatus::Failed(_)
            ));
        }
    }
}
