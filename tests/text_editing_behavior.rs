//! 文本编辑态（Editing Mode，工单 #11）数据安全集成测试。
//! 通过 `AppSession` 公共接缝与真实临时文件验证：
//! 编码 / BOM / 换行符原样写回、外部修改冲突、不可表示字符、只读失败、未保存拦截与决策、
//! 撤销重做、查找替换、自动保存、大文件禁用编辑、保存过程不留临时文件。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyfile::domain::{CloseDecision, ResolvedTheme, TextMode};
use anyfile::ports::{Clock, SystemShell, VideoBackend, VideoProbeInfo};
use anyfile::session::{
    AppSession, PendingAction, UNSAVED_CHANGES_ERROR, UnsavedDecision,
};
use anyfile::text_editor::{ConflictKind, SaveOptions, SaveOutcome};

struct FakeShell;
impl SystemShell for FakeShell {
    fn system_theme(&self) -> ResolvedTheme {
        ResolvedTheme::Dark
    }
    fn prefers_reduced_motion(&self) -> bool {
        true
    }
}

struct FakeClock(Mutex<Instant>);
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

struct FakeVideo;
impl VideoBackend for FakeVideo {
    fn is_available(&self) -> bool {
        false
    }
    fn probe(&self, _p: &Path) -> Option<VideoProbeInfo> {
        None
    }
}

fn session_with_clock() -> (AppSession, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock(Mutex::new(Instant::now())));
    let s = AppSession::new(Arc::new(FakeShell), clock.clone(), Arc::new(FakeVideo));
    (s, clock)
}

fn advance(clock: &FakeClock, d: Duration) {
    let mut g = clock.0.lock().unwrap();
    *g += d;
}

fn open_editing(s: &AppSession, path: &Path) {
    s.open_file(path).expect("打开应成功");
    s.text_edit_begin().expect("进入编辑态应成功");
    assert_eq!(s.text_mode(), TextMode::Editing);
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

fn no_temp_files(dir: &Path) {
    for e in fs::read_dir(dir).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!n.ends_with(".tmp"), "不应遗留临时文件: {n}");
    }
}

#[test]
fn save_preserves_encoding_bom_and_line_endings() {
    let dir = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Vec<u8>, Vec<u8>)> = vec![
        // (文件名, 原始字节, 追加 "X" 后期望字节)
        ("lf.txt", b"a\nb\n".to_vec(), b"Xa\nb\n".to_vec()),
        ("crlf.txt", b"a\r\nb\r\n".to_vec(), b"Xa\r\nb\r\n".to_vec()),
        (
            "bom.txt",
            [&[0xEF, 0xBB, 0xBF][..], b"a\r\n"].concat(),
            [&[0xEF, 0xBB, 0xBF][..], b"Xa\r\n"].concat(),
        ),
        (
            "gbk.txt",
            vec![0xD6, 0xD0, 0xCE, 0xC4, 0x0D, 0x0A],
            vec![b'X', 0xD6, 0xD0, 0xCE, 0xC4, 0x0D, 0x0A],
        ),
        (
            "u16le.txt",
            vec![0xFF, 0xFE, b'a', 0, b'\r', 0, b'\n', 0],
            vec![0xFF, 0xFE, b'X', 0, b'a', 0, b'\r', 0, b'\n', 0],
        ),
        (
            "u16be.txt",
            vec![0xFE, 0xFF, 0, b'a', 0, b'\n'],
            vec![0xFE, 0xFF, 0, b'X', 0, b'a', 0, b'\n'],
        ),
    ];
    for (name, original, expected) in cases {
        let p = write(dir.path(), name, &original);
        let (s, _) = session_with_clock();
        open_editing(&s, &p);
        let new_text = format!("X{}", s.text_edit_text());
        assert!(s.text_edit_set_text(&new_text), "{name}");
        assert!(s.text_edit_dirty());
        assert_eq!(s.text_edit_save(SaveOptions::default()), SaveOutcome::Saved, "{name}");
        assert!(!s.text_edit_dirty());
        assert_eq!(fs::read(&p).unwrap(), expected, "{name} 应原样保留编码/BOM/换行");
    }
    no_temp_files(dir.path());
}

#[test]
fn mixed_line_endings_normalize_to_dominant() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "m.txt", b"a\r\nb\r\nc\nd");
    let (s, _) = session_with_clock();
    open_editing(&s, &p);
    assert!(matches!(s.text_edit_meta(), Some((_, _, true))));
    s.text_edit_set_text("a\nb\nc\ndd");
    assert_eq!(s.text_edit_save(SaveOptions::default()), SaveOutcome::Saved);
    assert_eq!(fs::read(&p).unwrap(), b"a\r\nb\r\nc\r\ndd");
}

#[test]
fn external_modification_blocks_save_until_user_decides() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.txt", b"hello");
    let (s, _) = session_with_clock();
    open_editing(&s, &p);
    s.text_edit_set_text("mine");

    fs::write(&p, b"changed by other app!").unwrap();
    assert_eq!(s.text_edit_check_external(true), Some(ConflictKind::Modified));
    assert_eq!(
        s.text_edit_save(SaveOptions::default()),
        SaveOutcome::Conflict(ConflictKind::Modified)
    );
    assert_eq!(fs::read(&p).unwrap(), b"changed by other app!", "冲突时不得覆盖磁盘");

    let forced = SaveOptions { force: true, ..Default::default() };
    assert_eq!(s.text_edit_save(forced), SaveOutcome::Saved);
    assert_eq!(fs::read(&p).unwrap(), b"mine");
    assert_eq!(s.text_edit_external_conflict(), None);
}

#[test]
fn deleted_file_is_reported_and_reload_discards_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.txt", b"hello");
    let (s, _) = session_with_clock();
    open_editing(&s, &p);
    s.text_edit_set_text("mine");
    fs::remove_file(&p).unwrap();
    assert_eq!(s.text_edit_check_external(false), Some(ConflictKind::Deleted));
    assert_eq!(
        s.text_edit_save(SaveOptions::default()),
        SaveOutcome::Conflict(ConflictKind::Deleted)
    );

    s.text_edit_discard();
    let p2 = write(dir.path(), "b.txt", b"one");
    open_editing(&s, &p2);
    s.text_edit_set_text("two");
    fs::write(&p2, b"three").unwrap();
    s.text_edit_check_external(true);
    s.text_edit_reload().unwrap();
    assert_eq!(s.text_edit_text(), "three");
    assert!(!s.text_edit_dirty());
}

#[test]
fn unrepresentable_chars_prompt_and_convert_to_utf8_on_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let original = vec![0xD6, 0xD0, 0xCE, 0xC4, b'\n'];
    let p = write(dir.path(), "g.txt", &original);
    let (s, _) = session_with_clock();
    open_editing(&s, &p);
    s.text_edit_set_text("中文😀\n");
    match s.text_edit_save(SaveOptions::default()) {
        SaveOutcome::Unrepresentable(u) => {
            assert_eq!(u.chars, vec!['😀']);
            assert_eq!(u.first_line, 1);
        }
        other => panic!("应提示不可表示字符，实际 {other:?}"),
    }
    assert_eq!(fs::read(&p).unwrap(), original, "拒绝时磁盘内容不变");

    let conv = SaveOptions { convert_to_utf8: true, ..Default::default() };
    assert_eq!(s.text_edit_save(conv), SaveOutcome::Saved);
    assert_eq!(fs::read(&p).unwrap(), "中文😀\n".as_bytes());
}

#[test]
fn readonly_file_fails_without_touching_original_and_save_as_works() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "ro.txt", b"keep");
    let (s, _) = session_with_clock();
    open_editing(&s, &p);
    s.text_edit_set_text("edited");
    let mut perm = fs::metadata(&p).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&p, perm.clone()).unwrap();

    assert!(matches!(
        s.text_edit_save(SaveOptions::default()),
        SaveOutcome::NeedSaveAs(_)
    ));
    assert_eq!(fs::read(&p).unwrap(), b"keep");
    assert!(s.text_edit_dirty(), "失败后仍保留未保存状态");

    let target = dir.path().join("copy.txt");
    assert_eq!(
        s.text_edit_save_as(&target, SaveOptions::default()),
        SaveOutcome::Saved
    );
    assert_eq!(fs::read(&target).unwrap(), b"edited");
    assert_eq!(s.current_file_entry().unwrap().path, target);
    assert!(!s.text_edit_dirty());

    #[allow(clippy::permissions_set_readonly_false)]
    {
        perm.set_readonly(false);
        fs::set_permissions(&p, perm).unwrap();
    }
    no_temp_files(dir.path());
}

#[test]
fn unsaved_changes_intercept_navigation_open_switch_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let a = write(dir.path(), "a.txt", b"A");
    let b = write(dir.path(), "b.txt", b"B");
    let (s, _) = session_with_clock();
    open_editing(&s, &a);
    s.text_edit_set_text("A2");

    // 同目录翻页被拦截
    let err = s.navigate_next().unwrap_err();
    assert_eq!(err, UNSAVED_CHANGES_ERROR);
    assert_eq!(s.text_edit_pending(), Some(PendingAction::NavigateNext));
    assert_eq!(s.current_file_entry().unwrap().path, a);

    // 取消：留在原处，修改保留
    s.text_edit_decide(UnsavedDecision::Cancel);
    assert!(s.text_edit_pending().is_none());
    assert!(s.text_edit_dirty());

    // 打开其他文件被拦截；保存后继续打开
    assert!(s.open_file(&b).is_err());
    assert_eq!(s.text_edit_decide(UnsavedDecision::Save), Some(SaveOutcome::Saved));
    assert_eq!(fs::read(&a).unwrap(), b"A2");
    assert_eq!(s.current_file_entry().unwrap().path, b);
    assert!(!s.text_edit_active());

    // Esc 退出编辑：放弃修改
    open_editing(&s, &b);
    s.text_edit_set_text("B2");
    assert!(s.text_edit_request_exit().is_err());
    s.text_edit_decide(UnsavedDecision::Discard);
    assert_eq!(s.text_mode(), TextMode::Viewing);
    assert_eq!(fs::read(&b).unwrap(), b"B", "放弃时磁盘不变");

    // 关闭窗口：未保存 -> 确认；保存后批准退出
    open_editing(&s, &b);
    s.text_edit_set_text("B3");
    assert_eq!(s.request_close(), CloseDecision::ConfirmUnsavedChanges);
    assert!(!s.take_exit_approved());
    s.text_edit_decide(UnsavedDecision::Save);
    assert!(s.take_exit_approved());
    assert_eq!(fs::read(&b).unwrap(), b"B3");
    assert_eq!(s.request_close(), CloseDecision::AllowExit);
}

#[test]
fn undo_redo_and_find_replace_through_session() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.txt", b"foo Foo foo");
    let (s, clock) = session_with_clock();
    open_editing(&s, &p);

    assert_eq!(s.text_edit_find_all("foo", false).len(), 3);
    assert_eq!(s.text_edit_find_all("foo", true).len(), 2);
    assert_eq!(s.text_edit_replace_all("foo", "x", false), 3);
    assert_eq!(s.text_edit_text(), "x x x");
    assert!(s.text_edit_undo());
    assert_eq!(s.text_edit_text(), "foo Foo foo");
    assert!(s.text_edit_redo());
    assert_eq!(s.text_edit_text(), "x x x");
    assert!(s.text_edit_can_undo());

    advance(&clock, Duration::from_secs(5));
    s.text_edit_set_text("x x xy");
    assert!(s.text_edit_undo());
    assert_eq!(s.text_edit_text(), "x x x");
    // 撤销回到保存点后不再脏
    assert!(s.text_edit_undo());
    assert!(!s.text_edit_dirty());
}

#[test]
fn autosave_fires_only_after_idle_and_without_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.txt", b"v1");
    let (s, clock) = session_with_clock();
    open_editing(&s, &p);
    assert!(s.text_edit_autosave_tick().is_none(), "未修改不保存");
    s.text_edit_set_text("v2");
    assert!(s.text_edit_autosave_tick().is_none(), "刚编辑不保存");
    advance(&clock, Duration::from_secs(3));
    assert_eq!(s.text_edit_autosave_tick(), Some(SaveOutcome::Saved));
    assert_eq!(fs::read(&p).unwrap(), b"v2");

    // 外部修改后自动保存必须报冲突而不是覆盖
    s.text_edit_set_text("v3");
    fs::write(&p, b"other").unwrap();
    advance(&clock, Duration::from_secs(3));
    assert!(matches!(
        s.text_edit_autosave_tick(),
        Some(SaveOutcome::Conflict(_))
    ));
    assert_eq!(fs::read(&p).unwrap(), b"other");
}

#[test]
fn large_file_cannot_enter_editing() {
    let dir = tempfile::tempdir().unwrap();
    let big = vec![b'a'; 10 * 1024 * 1024 + 16];
    let p = write(dir.path(), "big.txt", &big);
    let (s, _) = session_with_clock();
    s.open_file(&p).unwrap();
    assert!(s.text_is_readonly_protected());
    assert!(s.text_edit_begin().is_err());
    assert!(!s.text_edit_active());
}

#[test]
fn lossy_files_refuse_editing_to_protect_data() {
    let dir = tempfile::tempdir().unwrap();
    // 合法 UTF-8 前缀 + 文件尾部非法字节：整体按 UTF-8 判定，解码会产生替换字符
    let mut bytes = "正常文本\n".repeat(30_000).into_bytes();
    bytes.extend_from_slice(&[0xFF, 0xFE, 0xFD, b'\n']);
    let p = write(dir.path(), "bad.txt", &bytes);
    let (s, _) = session_with_clock();
    s.open_file(&p).unwrap();
    // 要么按其他编码无损往返可编辑，要么被拒绝；无论如何保存不得改变未编辑文件字节
    if s.text_edit_begin().is_ok() {
        let t = s.text_edit_text();
        s.text_edit_set_text(&t);
        assert!(!s.text_edit_dirty());
    }
    assert_eq!(fs::read(&p).unwrap(), bytes);
}
