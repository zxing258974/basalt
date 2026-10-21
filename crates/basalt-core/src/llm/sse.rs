//! Server-Sent Events 的增量解析:网络上来一块喂一块,凑够一个事件就交出去。
//! 行可能被切在两块中间(连同一个汉字的几个字节),所以按字节缓存到换行再解码。
//! <https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation>

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SseEvent {
    /// `event:` 行;没有时为 `None`(OpenAI 的流只有 data)。
    pub event: Option<String>,
    /// 多行 `data:` 用换行连起来。
    pub data: String,
}

#[derive(Default)]
pub(crate) struct SseParser {
    pending: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    /// 流开头的 UTF-8 BOM 已经看过(去掉了或者没有)。
    started: bool,
}

impl SseParser {
    /// 喂进一块字节,返回这块里凑齐的事件。
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);
        if !self.started {
            const BOM: &[u8] = b"\xEF\xBB\xBF";
            if self.pending.len() < BOM.len() && BOM.starts_with(&self.pending) {
                return Vec::new();
            }
            if self.pending.starts_with(BOM) {
                self.pending.drain(..BOM.len());
            }
            self.started = true;
        }
        let mut events = Vec::new();
        // 行可以以 CRLF、LF 或单独的 CR 结尾;CR 恰好是这一块的最后一个字节时,等下一块看后面是不是 LF。
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n' || b == b'\r') {
            let crlf = self.pending[end] == b'\r' && self.pending.get(end + 1) == Some(&b'\n');
            if self.pending[end] == b'\r' && end + 1 == self.pending.len() {
                break;
            }
            let line: Vec<u8> = self.pending.drain(..end).collect();
            self.pending.drain(..if crlf { 2 } else { 1 });
            if let Some(event) = self.line(&String::from_utf8_lossy(&line)) {
                events.push(event);
            }
        }
        events
    }

    /// 流结束:最后一个事件后面没有空行时也交出去。
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            let rest = String::from_utf8_lossy(&rest).into_owned();
            if let Some(event) = self.line(rest.trim_end_matches('\r')) {
                return Some(event);
            }
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        let event = SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data).join("\n"),
        };
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: name.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    #[test]
    fn events_split_across_chunks_and_inside_a_character() {
        let stream = "event: content_block_delta\r\ndata: {\"text\":\"查询\"}\r\n\r\n: ping\n\ndata: a\ndata: b\n\n";
        let bytes = stream.as_bytes();
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        // 一个字节一个字节地喂:「查」的三个字节分在三块里。
        for byte in bytes {
            events.extend(parser.feed(std::slice::from_ref(byte)));
        }
        assert_eq!(
            events,
            [
                event(Some("content_block_delta"), "{\"text\":\"查询\"}"),
                event(None, "a\nb"),
            ]
        );
        assert_eq!(parser.finish(), None);
    }

    #[test]
    fn bare_carriage_returns_and_a_leading_bom_are_understood() {
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        // BOM 被切成两块;CR 单独结尾,也在块边上。
        for chunk in [&b"\xEF\xBB"[..], b"\xBFdata: a\r", b"\rdata: b\r\n\r\n"] {
            events.extend(parser.feed(chunk));
        }
        assert_eq!(events, [event(None, "a"), event(None, "b")]);
    }

    #[test]
    fn a_last_event_without_a_blank_line_is_still_delivered() {
        let mut parser = SseParser::default();
        assert!(parser.feed(b"data: [DONE]").is_empty());
        assert_eq!(parser.finish(), Some(event(None, "[DONE]")));
        let mut parser = SseParser::default();
        assert_eq!(parser.feed(b"data:no-space\n"), []);
        assert_eq!(parser.finish(), Some(event(None, "no-space")));
    }
}
