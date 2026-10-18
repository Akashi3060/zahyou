//! 締め切りと中止(Python の `_SvClock`)と、探索の中で使う止まり方。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// 探索を途中でやめる理由(外へは `SolveError` か `None` にして返す)。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Abort {
    /// 中止の合図(Python の SvCancelled)
    Cancelled,
    /// 締め切りを過ぎた(Python の _SvTimeUp)
    TimeUp,
    /// 中身の壊れた索引(Python の SvIndexCorrupt)。files の番号と理由(「名前: 理由」)
    Corrupt { file: usize, msg: String },
    /// 索引を今は開けない(OS のエラー。ほかのプロセスが書き込み用に開いている・権限が無いなど)。files の番号と理由。
    /// 中身が壊れているとは限らないので、その探索の中でだけ外し、次の試行では開き直す
    Unavailable { file: usize, msg: String },
    /// 入力がおかしい(Python の ValueError。ヒントの値・スケールなど)
    Invalid(String),
}

/// 締め切りと中止を確かめる。
#[derive(Clone, Copy)]
pub(crate) struct Clock<'a> {
    pub deadline: Option<Instant>,
    pub cancel: &'a AtomicBool,
}

impl<'a> Clock<'a> {
    pub fn new(deadline: Option<Instant>, cancel: &'a AtomicBool) -> Clock<'a> {
        Clock { deadline, cancel }
    }

    /// 中止なら Cancelled、締め切りを過ぎたら TimeUp(中止を先に見る。Python と同じ)。
    #[inline]
    pub fn check(&self) -> Result<(), Abort> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Abort::Cancelled);
        }
        if let Some(d) = self.deadline
            && Instant::now() >= d
        {
            return Err(Abort::TimeUp);
        }
        Ok(())
    }

    /// 締め切りを外した同じ時計(中止は見る)。
    pub fn without_deadline(&self) -> Clock<'a> {
        Clock { deadline: None, cancel: self.cancel }
    }

    /// 並列の処理が止まったとき、中止の合図が立っていれば中止を優先する(締め切りと同時のとき)。
    pub fn prefer_cancel(&self, e: Abort) -> Abort {
        if e == Abort::TimeUp && self.cancel.load(Ordering::Relaxed) { Abort::Cancelled } else { e }
    }
}
