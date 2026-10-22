//! 一个「够用就好」的 JSON 解析器。
//!
//! 在线 OCR 的响应要解析成 [`Json`]，而本项目的依赖表里没有 `serde_json`（只有 `serde` 本身，
//! 它不负责解析文本）。为了一个功能把依赖树撑大不划算，所以这里自己实现需要的那部分：
//! 对象、数组、字符串、数字、布尔、`null`，字符串支持 `\uXXXX` 转义（含代理对）。
//!
//! 不做的：流式解析、序列化、错误恢复、性能优化。够读 PaddleOCR 那种响应就行。
//!
//! ```
//! use cuter_core::ocr::json;
//! let value = json::parse(r#"{"results": [[["你好", 0.98]]]}"#).unwrap();
//! let text = value.get("results").and_then(|r| r.as_array())
//!     .and_then(|outer| outer.first()).and_then(|inner| inner.as_array())
//!     .and_then(|items| items.first()).and_then(|item| item.as_array())
//!     .and_then(|entry| entry.first()).and_then(|t| t.as_str());
//! assert_eq!(text, Some("你好"));
//! ```

/// 一个 JSON 值。
///
/// 对象用 `Vec` 而不是 `HashMap`：响应里的字段往往很少，保持插入顺序比 O(1) 查找更实用，
/// 也让 `Debug` 输出和原文一致、便于断言。
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Self>),
    Obj(Vec<(String, Self)>),
}

impl Json {
    /// 取对象字段；不是对象、或没有这个字段时返回 `None`。
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Obj(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Arr(items) => Some(items),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Num(number) => Some(*number),
            _ => None,
        }
    }
}

/// 解析一段 JSON 文本。
///
/// 出错时返回带字节位置的中文说明 —— 这个字符串最终会显示在识别面板里，
/// 所以要能让人看出"是服务端返回的东西不对"还是"服务端根本不是 OCR 接口"。
pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.pos != parser.bytes.len() {
        return Err(format!("第 {} 字节之后还有多余内容", parser.pos));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("第 {} 字节处应当是 '{}'", self.pos, char::from(byte)))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(_) => self.number(),
            None => Err(format!("第 {} 字节处意外结束", self.pos)),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(format!("第 {} 字节处应当是 {word}", self.pos))
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(entries));
                }
                _ => return Err(format!("第 {} 字节处的对象少了 ',' 或 '}}'", self.pos)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(format!("第 {} 字节处的数组少了 ',' 或 ']'", self.pos)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(format!("第 {} 字节处的字符串没有收尾", self.pos));
            };
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(format!("第 {} 字节处的转义没有收尾", self.pos));
                    };
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push_str(&self.unicode_escape()?),
                        other => {
                            return Err(format!(
                                "第 {} 字节处不认识的转义 \\{}",
                                self.pos,
                                char::from(other)
                            ));
                        }
                    }
                }
                // UTF-8 多字节字符：按首字节算出长度，整段取出来
                _ => {
                    let start = self.pos - 1;
                    let end = (start + utf8_len(byte)).min(self.bytes.len());
                    match std::str::from_utf8(&self.bytes[start..end]) {
                        Ok(text) => {
                            out.push_str(text);
                            self.pos = end;
                        }
                        Err(_) => return Err(format!("第 {start} 字节处不是合法的 UTF-8")),
                    }
                }
            }
        }
    }

    /// `\u` 之后的四位十六进制；高代理后面还要再吃一个 `\uDC00`-`\uDFFF`。
    fn unicode_escape(&mut self) -> Result<String, String> {
        let first = self.hex4()?;
        let code = if (0xD800..=0xDBFF).contains(&first) {
            if self.peek() == Some(b'\\') {
                self.pos += 1;
            } else {
                return Err(format!("第 {} 字节处的高代理后面缺少低代理", self.pos));
            }
            if self.peek() == Some(b'u') {
                self.pos += 1;
            } else {
                return Err(format!("第 {} 字节处的高代理后面缺少 \\u", self.pos));
            }
            let second = self.hex4()?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err(format!("第 {} 字节处的低代理不合法", self.pos));
            }
            0x1_0000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00)
        } else {
            u32::from(first)
        };
        // 落单的代理项在 UTF-8 里本来就无法表示，退回替换字符而不是报错
        Ok(char::from_u32(code).unwrap_or('\u{fffd}').to_string())
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err(format!("第 {} 字节处的 \\u 缺数字", self.pos));
            };
            let digit = char::from(byte).to_digit(16);
            let Some(digit) = digit else {
                return Err(format!("第 {} 字节处 \\u 后面不是十六进制", self.pos));
            };
            value = value * 16 + digit as u16;
            self.pos += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| format!("第 {start} 字节处的数字不是 UTF-8"))?;
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("第 {start} 字节处的数字看不懂：{text}"))
    }
}

/// UTF-8 首字节 → 这个字符占几个字节（非法首字节按 1 处理，解析会在别处报错）。
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_nested_objects_and_arrays() {
        let value = parse(r#"{"a": [1, 2.5, "三"], "b": {"c": true, "d": null}}"#).expect("parse");
        let a = value.get("a").and_then(Json::as_array).expect("a");
        assert_eq!(a[0].as_f64(), Some(1.0));
        assert_eq!(a[1].as_f64(), Some(2.5));
        assert_eq!(a[2].as_str(), Some("三"));
        assert_eq!(value.get("b").and_then(|b| b.get("c")), Some(&Json::Bool(true)));
        assert_eq!(value.get("b").and_then(|b| b.get("d")), Some(&Json::Null));
        assert_eq!(value.get("missing"), None);
    }

    #[test]
    fn it_understands_escapes_including_surrogate_pairs() {
        let value = parse(r#""引号\" 反斜杠\\ 换行\n 制表\t 汉字\u4f60\u597d 笑脸\ud83d\ude00""#)
            .expect("parse");
        assert_eq!(
            value.as_str(),
            Some("引号\" 反斜杠\\ 换行\n 制表\t 汉字你好 笑脸😀")
        );
    }

    #[test]
    fn it_tolerates_whitespace_and_empty_containers() {
        assert_eq!(parse("  {  }  ").expect("parse"), Json::Obj(Vec::new()));
        assert_eq!(parse("[]").expect("parse"), Json::Arr(Vec::new()));
        assert_eq!(
            parse("{\n\t\"a\" :\t[ ]\n}").expect("parse"),
            Json::Obj(vec![("a".to_owned(), Json::Arr(Vec::new()))])
        );
    }

    #[test]
    fn it_reports_what_is_wrong_with_a_bad_document() {
        // 不是 JSON 的正文（比如把 HTML 错误页当响应）
        assert!(parse("<html>404</html>").is_err());
        // 少收尾
        assert!(parse(r#"{"a": 1"#).is_err());
        // 多余内容
        assert!(parse("{} {}").is_err());
        // 坏转义
        assert!(parse(r#""\x""#).is_err());
        // 落的低代理
        assert!(parse(r#""\ud83d""#).is_err());
    }

    #[test]
    fn it_keeps_utf8_text_whole() {
        let value = parse(r#"{"text": "中文标点：「」，。！"}"#).expect("parse");
        assert_eq!(
            value.get("text").and_then(Json::as_str),
            Some("中文标点：「」，。！")
        );
    }
}
