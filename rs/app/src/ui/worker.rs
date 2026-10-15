//! 重い処理の受け渡し(Python 版の _bg / _post / _pump に当たる)。
//!
//! - 重い処理はワーカー 1 本だけ(busy)。二重に走らせない
//! - ワーカー → 画面は mpsc の enum 1 本(ログ 1 行・進捗・結果・終わりの一言)。キューの順番に頼らず、
//!   終わりの一言はワーカーの戻り値で明示的に渡す(v6 は lambda が実行時の値を読む作りだった)
//! - 中止は Arc<AtomicBool>。解析・ダウンロード・更新が同じものを見る
//! - ワーカーの panic は catch_unwind で受けて、押した場所のログへ書く(画面は落ちない)

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use eframe::egui;

use super::bridge::ResultView;
use super::survey::Survey;

/// ログの書き先(押した場所のログへ出す)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogTo {
    /// [解析] タブのログ
    Run,
    /// [準備] タブのログ
    Env,
}

/// ワーカー → 画面
pub enum Msg {
    Log(LogTo, String),
    /// ダウンロードの進み具合 (済み, 全体) [byte]
    Progress(u64, u64),
    /// 解析の結果
    Run(Box<ResultView>),
    /// 星図データの状態
    Env(Box<Survey>),
    /// 更新の確認の結果(quiet = 起動したついでの確認)
    Update(Box<zahyou_net::UpdateCheck>, bool),
    /// 段の一覧に付ける印
    Marks(Vec<u32>),
    /// 更新のために閉じる
    QuitForUpdate,
    /// 図の保存の結果
    Saved(Result<String, String>),
    /// 重い処理が終わった(状態バーに残す一言)
    Done(String),
    /// 重い処理が失敗した(panic など)
    Failed { to: LogTo, msg: String, detail: Vec<String> },
}

/// ワーカーに渡す道具(ログ・進捗・中止)
#[derive(Clone)]
pub struct Job {
    tx: Sender<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub to: LogTo,
    ctx: egui::Context,
}

impl Job {
    pub fn new(tx: Sender<Msg>, cancel: Arc<AtomicBool>, to: LogTo, ctx: egui::Context) -> Job {
        Job { tx, cancel, to, ctx }
    }
    /// 画面へ送る(送れなければ = 画面が閉じたあとなら、黙って捨てる)
    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
    /// 押した場所のログへ 1 行
    pub fn log(&self, s: &str) {
        self.send(Msg::Log(self.to, s.to_string()));
    }
    pub fn progress(&self, done: u64, total: u64) {
        self.send(Msg::Progress(done, total));
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

thread_local! {
    /// panic の中身(フックが書き、catch_unwind のあとで読む)
    static LAST_PANIC: RefCell<Option<(String, Vec<String>)>> = const { RefCell::new(None) };
}

/// panic のフックを付ける(main で 1 回)。中身をそのスレッドに覚えさせ、既定のフックにも渡す。
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = panic_message(info.payload());
        let mut detail = Vec::new();
        if let Some(loc) = info.location() {
            detail.push(format!("場所: {}:{}:{}", loc.file(), loc.line(), loc.column()));
        }
        if let Some(name) = std::thread::current().name() {
            detail.push(format!("スレッド: {name}"));
        }
        LAST_PANIC.with(|p| *p.borrow_mut() = Some((msg, detail)));
        default(info);
    }));
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "(中身の分からない panic)".to_string()
    }
}

/// `f` を panic から守って動かす。panic なら (メッセージ, 補足) を返す。
pub fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, (String, Vec<String>)> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Ok(r),
        Err(payload) => {
            let got = LAST_PANIC.with(|p| p.borrow_mut().take());
            Err(got.unwrap_or_else(|| (panic_message(payload.as_ref()), Vec::new())))
        }
    }
}

/// 重い処理を 1 本走らせる。終わったら Done(終わりの一言)か Failed を送る。
pub fn spawn_busy(job: Job, name: &str, f: impl FnOnce(&Job) -> String + Send + 'static) {
    let builder = std::thread::Builder::new().name(name.to_string()).stack_size(16 << 20);
    let j2 = job.clone();
    let spawned = builder.spawn(move || {
        let r = guarded(|| f(&job));
        match r {
            Ok(final_msg) => job.send(Msg::Done(final_msg)),
            Err((msg, detail)) => {
                job.send(Msg::Failed { to: job.to, msg, detail });
                job.send(Msg::Done("失敗しました。ログを見てください。".into()));
            }
        }
    });
    if let Err(e) = spawned {
        j2.send(Msg::Failed { to: j2.to, msg: format!("スレッドを起こせません: {e}"), detail: Vec::new() });
        j2.send(Msg::Done("失敗しました。ログを見てください。".into()));
    }
}

/// busy の外で動かす軽い処理(起動したついでの確認など)。失敗はログへ書くだけ。
pub fn spawn_quiet(job: Job, name: &str, f: impl FnOnce(&Job) + Send + 'static) {
    let builder = std::thread::Builder::new().name(name.to_string()).stack_size(8 << 20);
    let _ = builder.spawn(move || {
        if let Err((msg, detail)) = guarded(|| f(&job)) {
            job.send(Msg::Failed { to: job.to, msg, detail });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_を受け止める() {
        install_panic_hook();
        let r: Result<i32, _> = guarded(|| panic!("わざと {}", 1));
        let (msg, detail) = r.unwrap_err();
        assert_eq!(msg, "わざと 1");
        assert!(detail.iter().any(|d| d.starts_with("場所: ") && d.contains("worker.rs")), "{detail:?}");
        assert_eq!(guarded(|| 7).unwrap(), 7);
    }

    #[test]
    fn ワーカーの_panic_は失敗と終わりの一言になる() {
        install_panic_hook();
        let (tx, rx) = std::sync::mpsc::channel();
        let job = Job::new(tx, Arc::new(AtomicBool::new(false)), LogTo::Env, egui::Context::default());
        spawn_busy(job, "t", |_| -> String { panic!("壊れた") });
        let mut got = Vec::new();
        while let Ok(m) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
            let done = matches!(m, Msg::Done(_));
            got.push(m);
            if done {
                break;
            }
        }
        assert!(got.iter().any(|m| matches!(m, Msg::Failed { to: LogTo::Env, msg, .. } if msg == "壊れた")));
        assert!(matches!(got.last(), Some(Msg::Done(s)) if s.contains("失敗しました")));
    }
}
