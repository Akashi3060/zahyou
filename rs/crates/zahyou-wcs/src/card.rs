//! FITS のヘッダーのカード 1 枚(キー・値・コメント)。画像の読み込みと WCS の書き出しで共用する。

/// カードの値。
#[derive(Clone, Debug, PartialEq)]
pub enum CardValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// 値の無いカード(COMMENT・HISTORY・空のキー)
    None,
}

/// FITS のカード。`key` は 8 文字以内の大文字(前後の空白は除いたもの)。
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub key: String,
    pub value: CardValue,
    pub comment: String,
}

impl Card {
    pub fn new(key: &str, value: CardValue, comment: &str) -> Card {
        Card { key: key.to_string(), value, comment: comment.to_string() }
    }

    /// 値の無い注記のカード(COMMENT・HISTORY)。文は `comment` に入れる。
    pub fn commentary(key: &str, text: &str) -> Card {
        Card { key: key.to_string(), value: CardValue::None, comment: text.to_string() }
    }

    /// 数値として読む(整数・実数のどちらでも)。
    pub fn as_f64(&self) -> Option<f64> {
        match &self.value {
            CardValue::Int(i) => Some(*i as f64),
            CardValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            CardValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

/// カードの並びから、最初に一致したキーを探す。
pub fn find_card<'a>(cards: &'a [Card], key: &str) -> Option<&'a Card> {
    cards.iter().find(|c| c.key == key)
}
