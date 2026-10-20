//! 文本文件的编码:导出时把文本编码成 UTF-8 / GBK,导入时探测编码并流式解码。
//! 两个方向都不做替换:GBK 表示不了的字符、解不开的字节一律报错,不悄悄换成别的字符。

use std::borrow::Cow;

use encoding_rs::{DecoderResult, Encoding, GBK, UTF_8};

use super::TextEncoding;

pub const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// 文件开头要写的字节。
pub fn preamble(encoding: TextEncoding) -> &'static [u8] {
    match encoding {
        TextEncoding::Utf8Bom => UTF8_BOM,
        TextEncoding::Utf8 | TextEncoding::Gbk => &[],
    }
}

/// 编码一段文本。GBK 表示不了的字符原样返回给调用方报错。
pub fn encode(encoding: TextEncoding, text: &str) -> Result<Cow<'_, [u8]>, char> {
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => Ok(Cow::Borrowed(text.as_bytes())),
        TextEncoding::Gbk => {
            let (bytes, _, unmappable) = GBK.encode(text);
            if !unmappable {
                return Ok(bytes);
            }
            let mut buffer = [0u8; 4];
            let bad = text
                .chars()
                .find(|c| GBK.encode(c.encode_utf8(&mut buffer)).2)
                .unwrap_or(char::REPLACEMENT_CHARACTER);
            Err(bad)
        }
    }
}

/// 看文件开头一段猜编码:有 BOM 或者是有效的 UTF-8 就是 UTF-8,否则当 GBK。
/// 样本可能在一个多字节字符中间截断,截断处不算错。
pub fn detect(sample: &[u8]) -> TextEncoding {
    if sample.starts_with(UTF8_BOM) {
        return TextEncoding::Utf8Bom;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => TextEncoding::Utf8,
        Err(error) if error.error_len().is_none() => TextEncoding::Utf8,
        Err(_) => TextEncoding::Gbk,
    }
}

fn encoding_of(encoding: TextEncoding) -> &'static Encoding {
    match encoding {
        TextEncoding::Utf8 | TextEncoding::Utf8Bom => UTF_8,
        TextEncoding::Gbk => GBK,
    }
}

/// 流式解码:一块一块喂字节,开头的 BOM 去掉。
pub struct Decoder {
    inner: encoding_rs::Decoder,
    encoding: TextEncoding,
    /// 已经解码的字节数,报错时说位置。
    consumed: u64,
}

impl Decoder {
    pub fn new(encoding: TextEncoding) -> Self {
        Self {
            inner: encoding_of(encoding).new_decoder_with_bom_removal(),
            encoding,
            consumed: 0,
        }
    }

    /// 把 `bytes` 解码追加到 `out`。`last` 表示文件读完了:末尾还有半个字符就是错。
    pub fn decode(&mut self, bytes: &[u8], last: bool, out: &mut String) -> Result<(), String> {
        let mut rest = bytes;
        loop {
            let needed = self
                .inner
                .max_utf8_buffer_length_without_replacement(rest.len())
                .unwrap_or(rest.len() * 3 + 16);
            out.reserve(needed);
            let (result, read) = self
                .inner
                .decode_to_string_without_replacement(rest, out, last);
            self.consumed += read as u64;
            rest = &rest[read..];
            match result {
                DecoderResult::InputEmpty => return Ok(()),
                DecoderResult::OutputFull => continue,
                DecoderResult::Malformed(..) => {
                    let name = match self.encoding {
                        TextEncoding::Gbk => "GBK",
                        TextEncoding::Utf8 | TextEncoding::Utf8Bom => "UTF-8",
                    };
                    return Err(format!(
                        "文件第 {} 个字节附近有不是 {name} 的内容，请换一种编码再试。",
                        self.consumed
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(encoding: TextEncoding, chunks: &[&[u8]]) -> Result<String, String> {
        let mut decoder = Decoder::new(encoding);
        let mut out = String::new();
        for (index, chunk) in chunks.iter().enumerate() {
            decoder.decode(chunk, index + 1 == chunks.len(), &mut out)?;
        }
        Ok(out)
    }

    #[test]
    fn gbk_round_trips_chinese_and_rejects_unmappable_characters() {
        let bytes = encode(TextEncoding::Gbk, "中文,abc").unwrap();
        assert_eq!(&bytes[..], b"\xD6\xD0\xCE\xC4,abc");
        assert_eq!(
            decode_all(TextEncoding::Gbk, &[&bytes]).unwrap(),
            "中文,abc"
        );
        assert_eq!(encode(TextEncoding::Gbk, "好😀").unwrap_err(), '😀');
        assert_eq!(
            encode(TextEncoding::Utf8, "😀").unwrap().as_ref(),
            "😀".as_bytes()
        );
    }

    #[test]
    fn detects_bom_utf8_and_falls_back_to_gbk() {
        assert_eq!(detect(b"\xEF\xBB\xBFid"), TextEncoding::Utf8Bom);
        assert_eq!(detect("名字".as_bytes()), TextEncoding::Utf8);
        // 样本在「字」的中间截断。
        assert_eq!(detect(&"名字".as_bytes()[..4]), TextEncoding::Utf8);
        assert_eq!(detect(b"\xD6\xD0\xCE\xC4"), TextEncoding::Gbk);
        assert_eq!(detect(b""), TextEncoding::Utf8);
        assert_eq!(preamble(TextEncoding::Utf8Bom), UTF8_BOM);
        assert!(preamble(TextEncoding::Gbk).is_empty());
    }

    #[test]
    fn decoder_handles_split_characters_strips_bom_and_reports_bad_bytes() {
        let text = "\u{feff}张三".as_bytes();
        // 「张」的三个字节被切在两块里。
        assert_eq!(
            decode_all(TextEncoding::Utf8, &[&text[..4], &text[4..]]).unwrap(),
            "张三"
        );
        let error = decode_all(TextEncoding::Utf8, &[b"ab\xD6\xD0"]).unwrap_err();
        assert!(error.contains("UTF-8"), "{error}");
        let truncated = decode_all(TextEncoding::Utf8, &[&"张".as_bytes()[..2]]).unwrap_err();
        assert!(truncated.contains("换一种编码"), "{truncated}");
    }
}
