//! 在线 OCR：把框选的图 POST 给自建服务（PaddleOCR 的 `hubserving` 形状）。
//!
//! * 请求：`multipart/form-data`，一个字段 `image`，内容是 PNG 的 base64
//!   （PaddleOCR 的 `read_image` 见到字符串会按 base64 解，这也是它服务端最常见的用法）；
//! * 响应：`{"results": [[[text, score, [[x, y] × 4]], …]], "status": "000"}`
//!   —— 外层数组对应"一次请求里的每张图"，我们只发一张，所以取第一个元素。
//!
//! ## 为什么自己写 HTTP
//!
//! 只发一个 POST、只读一个响应体，`std::net` 足够；为了这点事把 `reqwest`（连带 TLS、
//! 异步运行时）拉进依赖树不划算。代价是**只支持 `http://`**：没有 TLS 实现，
//! `https://` 会明确报错而不是静默失败（自建服务一般在局域网里跑 http，够用）。
//!
//! ## 配置里的 `min_score`
//!
//! 内置引擎拿不到逐块置信度，所以那个设置在那边不生效；在线接口能拿到，这里按它过滤。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::frame::{Frame, ImageFormat};
use crate::geom::Point;
use crate::ocr::{OcrEngine, OcrResult, TextBlock, json};

/// 默认超时：自建服务在局域网里通常几百毫秒，15 秒留足余量。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
/// 响应体上限：对端如果不是我们的服务，别让它把内存吃光。
const MAX_BODY: usize = 8 * 1024 * 1024;
/// multipart 分隔符：固定串即可，它只在这一个请求里出现，不必随机。
const BOUNDARY: &str = "----cuter-ocr-boundary";

/// 走 HTTP 的 OCR 引擎。
#[derive(Debug, Clone)]
pub struct HttpOcr {
    endpoint: String,
    timeout: Duration,
    min_score: f32,
}

impl HttpOcr {
    /// 用接口地址建一个引擎，地址形如 `http://127.0.0.1:8866/ocr`。
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout: DEFAULT_TIMEOUT,
            min_score: 0.0,
        }
    }

    #[must_use]
    pub fn with_min_score(mut self, min_score: f32) -> Self {
        self.min_score = min_score.clamp(0.0, 1.0);
        self
    }

    /// 发一次请求，返回响应体文本。
    fn post(&self, png: &[u8]) -> Result<String> {
        let (host, port, path) = split_endpoint(self.endpoint.trim())?;
        let payload = multipart_body(BOUNDARY, "image", &base64_encode(png));
        let request = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {host}:{port}\r\n\
             Content-Type: multipart/form-data; boundary={BOUNDARY}\r\n\
             Content-Length: {}\r\n\
             Accept: application/json\r\n\
             Connection: close\r\n\r\n",
            payload.len()
        );

        let address = resolve(&host, port)?;
        let mut stream = TcpStream::connect_timeout(&address, self.timeout)
            .map_err(|err| Error::Ocr(format!("连不上 OCR 接口 {host}:{port}：{err}")))?;
        let _ = stream.set_read_timeout(Some(self.timeout));
        let _ = stream.set_write_timeout(Some(self.timeout));
        stream
            .write_all(request.as_bytes())
            .and_then(|()| stream.write_all(payload.as_bytes()))
            .and_then(|()| stream.flush())
            .map_err(|err| Error::Ocr(format!("把图片发给 OCR 接口时失败：{err}")))?;

        let mut raw = Vec::new();
        stream
            .take(MAX_BODY as u64)
            .read_to_end(&mut raw)
            .map_err(|err| Error::Ocr(format!("读 OCR 接口的响应时失败：{err}")))?;
        let (status, body) = split_response(&String::from_utf8_lossy(&raw))?;
        if status != 200 {
            let head: String = body.trim().chars().take(200).collect();
            return Err(Error::Ocr(format!(
                "OCR 接口返回 HTTP {status}{}",
                if head.is_empty() {
                    String::new()
                } else {
                    format!("：{head}")
                }
            )));
        }
        Ok(body)
    }
}

impl OcrEngine for HttpOcr {
    fn name(&self) -> &str {
        "在线 OCR"
    }

    fn recognize(&self, image: &Frame) -> Result<OcrResult> {
        let started = Instant::now();
        let png = image.encode(ImageFormat::Png)?;
        let body = self.post(&png)?;
        let mut result = parse_paddle_response(&body, self.min_score)?;
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }

    fn is_ready(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }
}

/// 拆 `http://主机[:端口][/路径]`。
///
/// 只管这一种形状：不认用户信息、不认 IPv6 字面量（自建 OCR 服务用不上），
/// 但会**明确拒绝** `https://` —— 我们没有 TLS，与其连上去握手失败，不如直说。
pub fn split_endpoint(url: &str) -> Result<(String, u16, String)> {
    let Some(rest) = url.strip_prefix("http://") else {
        return Err(Error::Ocr(if url.starts_with("https://") {
            "在线 OCR 目前只支持 http://（没有 TLS 实现）：请换成局域网里的 http 地址".to_owned()
        } else {
            format!("OCR 接口地址要以 http:// 开头：{url}")
        }));
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(Error::Ocr(format!("OCR 接口地址里没有主机名：{url}")));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => (
            host.to_owned(),
            port.parse::<u16>()
                .map_err(|_| Error::Ocr(format!("OCR 接口地址里的端口不是数字：{port}")))?,
        ),
        _ => (authority.to_owned(), 80),
    };
    if host.is_empty() {
        return Err(Error::Ocr(format!("OCR 接口地址里没有主机名：{url}")));
    }
    Ok((host, port, path.to_owned()))
}

/// 解析主机名（`127.0.0.1` 直接过，域名走系统解析）。
fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    (host, port)
        .to_socket_addrs()
        .map_err(|err| Error::Ocr(format!("解析 OCR 接口主机名 {host} 失败：{err}")))?
        .next()
        .ok_or_else(|| Error::Ocr(format!("OCR 接口主机名 {host} 没有解析出地址")))
}

/// 把 HTTP 响应拆成状态码与正文。
pub fn split_response(raw: &str) -> Result<(u16, String)> {
    let Some(split) = raw.find("\r\n\r\n") else {
        return Err(Error::Ocr("OCR 接口的响应不是合法的 HTTP".to_owned()));
    };
    let (head, body) = (&raw[..split], &raw[split + 4..]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| Error::Ocr("OCR 接口的响应里没有状态码".to_owned()))?;
    // 分块传输：小服务一般直接给 Content-Length，但 Python/Flask 也常分块，顺手解开
    let chunked = head
        .lines()
        .skip(1)
        .any(|line| line.to_ascii_lowercase().starts_with("transfer-encoding:")
            && line.to_ascii_lowercase().contains("chunked"));
    let body = if chunked {
        dechunk(body).ok_or_else(|| Error::Ocr("OCR 接口的响应分块格式不对".to_owned()))?
    } else {
        body.to_owned()
    };
    Ok((status, body))
}

/// 解开 HTTP/1.1 的分块编码；格式不对时返回 `None`。
fn dechunk(body: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let (size_line, tail) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size_line.split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            return Some(out);
        }
        if tail.len() < size {
            return None;
        }
        out.push_str(&tail[..size]);
        rest = tail[size..].strip_prefix("\r\n").unwrap_or(&tail[size..]);
    }
}

/// multipart/form-data 的正文：一个字段、内容是 base64 文本。
#[must_use]
pub fn multipart_body(boundary: &str, field: &str, data: &str) -> String {
    format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"{field}\"\r\n\r\n\
         {data}\r\n\
         --{boundary}--\r\n"
    )
}

/// 标准 base64（带 `=` 补齐）。
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(ALPHABET[(triple >> 18) as usize & 0x3F]));
        out.push(char::from(ALPHABET[(triple >> 12) as usize & 0x3F]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(triple >> 6) as usize & 0x3F])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[triple as usize & 0x3F])
        } else {
            '='
        });
    }
    out
}

/// 解析 PaddleOCR `hubserving` 形状的响应。
///
/// 兼容两种箱子写法：`[text, score, [[x,y] × 4]]`（PaddleOCR 原样）与把四个点摊平在
/// 后面的 `[text, score, [x,y], [x,y], [x,y], [x,y]]`（有些封装会这么给）。
pub fn parse_paddle_response(body: &str, min_score: f32) -> Result<OcrResult> {
    let root = json::parse(body).map_err(|err| Error::Ocr(format!("OCR 接口返回的不是 JSON：{err}")))?;
    if let Some(status) = root.get("status").and_then(json::Json::as_str)
        && status != "000"
    {
        let message = root
            .get("msg")
            .and_then(json::Json::as_str)
            .unwrap_or("服务端没有给出原因");
        return Err(Error::Ocr(format!("OCR 服务返回失败（{status}）：{message}")));
    }
    let items = root
        .get("results")
        .and_then(json::Json::as_array)
        .and_then(|outer| outer.first())
        .and_then(json::Json::as_array)
        .ok_or_else(|| {
            Error::Ocr("OCR 接口的响应里没有 results[0] 数组（期望 PaddleOCR hubserving 形状）".to_owned())
        })?;

    let mut blocks = Vec::new();
    for item in items {
        let Some(entry) = item.as_array() else {
            continue;
        };
        let text = entry
            .first()
            .and_then(json::Json::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let score = entry.get(1).and_then(json::Json::as_f64).unwrap_or(0.0) as f32;
        if text.is_empty() || score < min_score {
            continue;
        }
        let corners: Vec<&json::Json> = match entry.get(2).and_then(json::Json::as_array) {
            Some(points) if points.first().is_some_and(|p| p.as_array().is_some()) => {
                points.iter().collect()
            }
            _ => entry.iter().skip(2).collect(),
        };
        let mut quad = [Point::ZERO; 4];
        let mut found = 0;
        for corner in corners {
            if found == 4 {
                break;
            }
            let Some(pair) = corner.as_array() else {
                continue;
            };
            let (Some(x), Some(y)) = (
                pair.first().and_then(json::Json::as_f64),
                pair.get(1).and_then(json::Json::as_f64),
            ) else {
                continue;
            };
            quad[found] = Point::new(x.round() as i32, y.round() as i32);
            found += 1;
        }
        if found < 4 {
            continue;
        }
        blocks.push(TextBlock::from_quad(quad, text, score));
    }
    Ok(OcrResult {
        blocks,
        elapsed_ms: 0,
    }
    .in_reading_order())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Rect, Size};
    use std::net::TcpListener;

    fn frame() -> Frame {
        let mut frame = Frame::new(Size::new(2, 2));
        frame.data_mut()[0..4].copy_from_slice(&[255, 0, 0, 255]);
        frame.data_mut()[4..8].copy_from_slice(&[0, 255, 0, 255]);
        frame
    }

    /// 读完整条 HTTP 请求（先头、再按 `Content-Length` 读正文）。
    fn read_request(stream: &mut TcpStream) -> String {
        let mut raw = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream.read(&mut buffer).expect("read");
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&raw);
            if let Some(split) = text.find("\r\n\r\n") {
                let length = text[..split]
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                if raw.len() >= split + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&raw).into_owned()
    }

    #[test]
    fn base64_matches_the_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // PNG 魔数，顺便确认不是把高位丢了
        assert_eq!(base64_encode(&[0x89, 0x50, 0x4E, 0x47]), "iVBORw==");
    }

    #[test]
    fn endpoint_splitting_covers_the_shapes_we_accept() {
        assert_eq!(
            split_endpoint("http://127.0.0.1:8866/ocr").expect("split"),
            ("127.0.0.1".to_owned(), 8866, "/ocr".to_owned())
        );
        assert_eq!(
            split_endpoint("http://localhost").expect("split"),
            ("localhost".to_owned(), 80, "/".to_owned())
        );
        assert_eq!(
            split_endpoint("http://192.168.1.9:8866/predict/ocr_system").expect("split"),
            ("192.168.1.9".to_owned(), 8866, "/predict/ocr_system".to_owned())
        );
        // https 必须给出明确理由，而不是握手失败
        let err = split_endpoint("https://example.com/ocr").expect_err("https");
        assert!(err.to_string().contains("http"), "{err}");
        assert!(split_endpoint("127.0.0.1:8866").is_err(), "缺协议头要报错");
        assert!(split_endpoint("http://host:notaport/ocr").is_err(), "端口要能解析");
    }

    #[test]
    fn multipart_body_carries_the_field_and_the_payload() {
        let body = multipart_body("BOUND", "image", "QUJD");
        assert!(body.starts_with("--BOUND\r\n"), "{body}");
        assert!(body.contains("Content-Disposition: form-data; name=\"image\""), "{body}");
        assert!(body.contains("\r\n\r\nQUJD\r\n"), "{body}");
        assert!(body.ends_with("--BOUND--\r\n"), "{body}");
    }

    #[test]
    fn it_reads_a_paddleocr_reply() {
        let body = r#"{"msg":"","results":[[["第一行",0.98,[[10,20],[60,20],[60,40],[10,40]]],
                                            ["第二行",0.12,[[10,50],[60,50],[60,70],[10,70]]]]],"status":"000"}"#;
        let result = parse_paddle_response(body, 0.3).expect("parse");
        assert_eq!(result.blocks.len(), 1, "0.12 的那块要被 min_score 滤掉");
        assert_eq!(result.blocks[0].text, "第一行");
        assert_eq!(result.blocks[0].bounds, Rect::new(10, 20, 50, 20));
        assert!((result.blocks[0].score - 0.98).abs() < 1e-6);
    }

    #[test]
    fn it_also_reads_the_flattened_box_shape() {
        let body = r#"{"results":[[["摊平的",0.9,[1,2],[11,2],[11,12],[1,12]]]],"status":"000"}"#;
        let result = parse_paddle_response(body, 0.0).expect("parse");
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].bounds, Rect::new(1, 2, 10, 10));
    }

    #[test]
    fn it_reports_server_side_failures_and_wrong_shapes() {
        let failed = r#"{"status":"100","msg":"image decode error"}"#;
        let err = parse_paddle_response(failed, 0.0).expect_err("status");
        assert!(err.to_string().contains("image decode error"), "{err}");

        let wrong = r#"{"hello":"world"}"#;
        assert!(parse_paddle_response(wrong, 0.0).is_err(), "没有 results 要报错");
        assert!(parse_paddle_response("<html>500</html>", 0.0).is_err(), "不是 JSON 要报错");
    }

    #[test]
    fn response_splitting_handles_chunked_bodies() {
        let plain = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(split_response(plain).expect("plain"), (200, "{}".to_owned()));

        let chunked = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"a\":1}\r\n0\r\n\r\n";
        assert_eq!(
            split_response(chunked).expect("chunked"),
            (200, "{\"a\":1}".to_owned())
        );

        assert!(split_response("not http at all").is_err());
    }

    /// 真发一次请求：本机起一个假服务端，断言请求里带着这张图的 base64、响应能读回来。
    #[test]
    fn it_posts_the_image_to_the_endpoint_and_reads_the_blocks() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_request(&mut stream);
            let body = r#"{"status":"000","results":[[["你好，世界",0.99,[[10,20],[60,20],[60,40],[10,40]]]]]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("write");
            request
        });

        let frame = frame();
        let png = frame.encode(ImageFormat::Png).expect("png");
        let engine = HttpOcr::new(format!("http://127.0.0.1:{port}/ocr"));
        let result = engine.recognize(&frame).expect("recognize");
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].text, "你好，世界");
        assert_eq!(result.blocks[0].bounds, Rect::new(10, 20, 50, 20));

        let request = server.join().expect("join");
        assert!(request.starts_with("POST /ocr HTTP/1.1\r\n"), "{request}");
        assert!(request.contains("Content-Type: multipart/form-data; boundary="), "{request}");
        assert!(request.contains("name=\"image\""), "{request}");
        assert!(
            request.contains(&base64_encode(&png)),
            "请求体里应当是这张 PNG 的 base64"
        );
    }

    /// 服务端返回 500 时，错误信息里要带状态码，方便用户判断是服务没起还是接口写错了。
    #[test]
    fn it_reports_a_non_200_status() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = read_request(&mut stream);
            let body = "boom";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });

        let engine = HttpOcr::new(format!("http://127.0.0.1:{port}/ocr"));
        let err = engine.recognize(&frame()).expect_err("should fail");
        let text = err.to_string();
        assert!(text.contains("500") && text.contains("boom"), "{text}");
    }

    /// 真实服务的响应（`crates/cuter-core/tests/fixtures/paddle_real_response.json`）：
    /// 本机 PaddleOCR 2.9.1 对 `docs/design/shots/0.1.49-ocr-row-hover.png` 的原样输出，
    /// 14 个块、词级四边形、含中文与全角标点。手写 fixture 只能证明"我们以为的形状"，
    /// 这个文件证明的是"真引擎给的形状"。
    #[test]
    fn it_parses_the_response_a_real_paddle_server_sent() {
        let body = include_str!("../../tests/fixtures/paddle_real_response.json");
        let result = parse_paddle_response(body, 0.3).expect("the real payload must parse");

        assert_eq!(result.blocks.len(), 14, "the fixture has 14 blocks");
        // 阅读顺序：最上面那一行是应用自己的面板标题。
        let first = &result.blocks[0];
        assert_eq!(first.text, "识别文本");
        assert!(
            first.score > 0.99,
            "the real score for that line is 0.9996, got {}",
            first.score
        );
        assert_eq!(first.bounds.x, 598);
        assert_eq!(first.bounds.y, 27);
        assert_eq!(first.bounds.w, 661 - 598);
        assert_eq!(first.bounds.h, 46 - 27);

        // 三行大字都在，且每个都带自己的四边形（不是整张图一个框）。
        for word in ["ALPHA", "BRAVO", "CHARLIE"] {
            assert!(
                result.blocks.iter().any(|block| block.text == word),
                "{word} missing from the real payload"
            );
        }
        // 中文 + 全角标点也必须原样保留（这是真引擎最容易暴露编码问题的地方）。
        assert!(
            result
                .blocks
                .iter()
                .any(|block| block.text.contains("单击图中的框可复制该行文字")),
            "the Chinese footer line was mangled"
        );
    }

    #[test]
    fn a_low_min_score_filters_the_real_payload() {
        let body = include_str!("../../tests/fixtures/paddle_real_response.json");
        let all = parse_paddle_response(body, 0.0).expect("parse");
        // 真实响应里最低分是 0.9559；把门槛抬到 0.99 应该只剩高分块。
        let strict = parse_paddle_response(body, 0.99).expect("parse");
        assert_eq!(all.blocks.len(), 14);
        assert!(
            strict.blocks.len() < all.blocks.len(),
            "a 0.99 threshold should drop some of the {} real blocks",
            all.blocks.len()
        );
        assert!(
            strict.blocks.iter().all(|block| block.score >= 0.99),
            "a block below the threshold survived"
        );
    }

    #[test]
    fn an_empty_endpoint_is_not_ready() {
        assert!(!HttpOcr::new("   ").is_ready());
        assert!(HttpOcr::new("http://127.0.0.1:8866/ocr").is_ready());
        assert_eq!(HttpOcr::new("").name(), "在线 OCR");
    }
}
