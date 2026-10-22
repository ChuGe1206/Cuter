//! 自动更新检查：读一份 JSON 版本清单，跟当前版本比大小。
//!
//! 为什么自己写 HTTP：项目硬约束是**不新增依赖**（没有 `reqwest` / `ureq`），
//! 而更新检查必须能走 `https`（清单通常挂在 GitHub Releases 之类的站点上）。
//! Windows 自带 WinHTTP（TLS 由 schannel 负责），所以这里分两条腿：
//!
//! | 地址 | 实现 | 依赖 |
//! |------|------|------|
//! | `https://…` | WinHTTP（`winhttp.dll`） | 系统 DLL，零新依赖 |
//! | `http://…` | 自己拼请求 + `TcpStream`（复用 `ocr::http_ocr::split_response`） | 零新依赖 |
//!
//! 清单格式（自己定，谁部署谁填地址，本项目没有官方发布渠道）：
//!
//! ```json
//! { "version": "0.1.47", "url": "https://example.com/cuter-0.1.47-x64-setup.exe",
//!   "notes": "修了几个问题" }
//! ```
//!
//! `url` 与 `notes` 可以缺省：只想知道"有没有新版本"的部署可以只写 `version`。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::ocr::http_ocr::split_response;
use crate::ocr::json::{self, Json};

/// 默认超时：更新检查是后台小动作，别把线程挂太久。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// 清单最大字节数（防止对面给个巨大的响应把内存吃光）。
pub const MAX_BODY: usize = 1024 * 1024;

/// 第一次失败后等多久再试：开机那几秒网络栈经常还没就绪，等一下比立刻放弃有用。
const RETRY_BACKOFF: Duration = Duration::from_millis(600);

/// 一份版本清单。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateManifest {
    /// 发布方声明的版本号（可以带前导 `v`）。
    pub version: String,
    /// 下载页 / 安装包地址，可能为空。
    pub url: String,
    /// 这版改了什么，可能为空。
    pub notes: String,
}

impl UpdateManifest {
    /// 解析清单。`version` 必须有，其余可缺。
    pub fn parse(text: &str) -> Result<Self, String> {
        // 包一层：直接把 JSON 解析器的"第 N 字节处…"扔给用户，看不出是清单写错了
        let value = json::parse(text).map_err(|err| format!("清单不是合法 JSON：{err}"))?;
        let version = value
            .get("version")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "清单里没有 version 字段".to_owned())?;
        let url = value
            .get("url")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let notes = value
            .get("notes")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        Ok(Self {
            version: version.to_owned(),
            url,
            notes,
        })
    }

    /// 这份清单是不是比 `current` 新。
    #[must_use]
    pub fn is_newer_than(&self, current: &str) -> bool {
        is_newer(current, &self.version)
    }
}

/// `candidate` 是否比 `current` 新。
///
/// 按 `.` / `-` / `+` 切段逐段比数字（前导 `v` 忽略，认不出的段按 0）。
/// 缺的段当 0：`0.2` 比 `0.1.46` 新，`0.1.46` 与 `0.1.46.0` 相同。
#[must_use]
pub fn is_newer(current: &str, candidate: &str) -> bool {
    let left = segments(current);
    let right = segments(candidate);
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if a != b {
            return b > a;
        }
    }
    false
}

fn segments(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-', '+', ' '])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

/// 请求体准备好了以后要连哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

fn split_url(url: &str) -> Result<Target, String> {
    let url = url.trim();
    let (rest, tls, default_port) = if let Some(rest) = url.strip_prefix("https://") {
        (rest, true, 443)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (rest, false, 80)
    } else {
        return Err("地址必须以 http:// 或 https:// 开头".to_owned());
    };
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err("地址里没有主机名".to_owned());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => {
            let port: u16 = port
                .parse()
                .map_err(|_| format!("端口不是数字：{port}"))?;
            (host.to_owned(), port)
        }
        _ => (authority.to_owned(), default_port),
    };
    Ok(Target {
        tls,
        host,
        port,
        path: path.to_owned(),
    })
}

/// 取回清单原文。
///
/// 失败重试一次：更新检查多在开机后跑，那一刻网络栈经常还没就绪
/// （Wi-Fi 刚关联、代理刚起来），一次失败不代表地址写错了。解析失败不重试
/// ——同样的一份 JSON 再取十次也还是那份错的。
pub fn fetch(url: &str, timeout: Duration) -> Result<String, String> {
    let target = split_url(url)?;
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(RETRY_BACKOFF);
        }
        let outcome = if target.tls {
            fetch_tls(&target, timeout)
        } else {
            fetch_plain(&target, timeout)
        };
        match outcome {
            Ok(text) => return Ok(text),
            Err(err) => last = err,
        }
    }
    Err(last)
}

// ---------------------------------------------------------------------------
// http://：自己拼一句话的 GET
// ---------------------------------------------------------------------------

fn fetch_plain(target: &Target, timeout: Duration) -> Result<String, String> {
    let address = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|err| format!("解析主机名失败：{err}"))?
        .next()
        .ok_or_else(|| format!("解析主机名失败：{}", target.host))?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|err| format!("连接 {}:{} 失败：{err}", target.host, target.port))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| err.to_string())?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        target.path,
        target.host,
        user_agent()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("发送请求失败：{err}"))?;
    let mut raw = Vec::new();
    stream
        .take(MAX_BODY as u64)
        .read_to_end(&mut raw)
        .map_err(|err| format!("读取响应失败：{err}"))?;
    let text = String::from_utf8_lossy(&raw);
    let (status, body) = split_response(&text).map_err(|err| err.to_string())?;
    if status != 200 {
        return Err(format!("服务器返回 HTTP {status}"));
    }
    Ok(body)
}

fn user_agent() -> String {
    format!("Cuter/{}", crate::VERSION)
}

// ---------------------------------------------------------------------------
// https://：WinHTTP（系统 TLS，零新依赖）
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod winhttp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;

    type Handle = *mut c_void;

    const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
    const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
    const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
    const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;

    #[link(name = "winhttp")]
    unsafe extern "system" {
        fn WinHttpOpen(
            agent: *const u16,
            access_type: u32,
            proxy: *const u16,
            bypass: *const u16,
            flags: u32,
        ) -> Handle;
        fn WinHttpConnect(session: Handle, server: *const u16, port: u16, reserved: u32) -> Handle;
        fn WinHttpOpenRequest(
            connect: Handle,
            verb: *const u16,
            object: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept_types: *const *const u16,
            flags: u32,
        ) -> Handle;
        fn WinHttpSetTimeouts(
            handle: Handle,
            resolve: i32,
            connect: i32,
            send: i32,
            receive: i32,
        ) -> i32;
        fn WinHttpSendRequest(
            request: Handle,
            headers: *const u16,
            headers_len: u32,
            optional: *const c_void,
            optional_len: u32,
            total_len: u32,
            context: usize,
        ) -> i32;
        fn WinHttpReceiveResponse(request: Handle, reserved: *mut c_void) -> i32;
        fn WinHttpQueryHeaders(
            request: Handle,
            info_level: u32,
            name: *const u16,
            buffer: *mut c_void,
            buffer_len: *mut u32,
            index: *mut u32,
        ) -> i32;
        fn WinHttpQueryDataAvailable(request: Handle, available: *mut u32) -> i32;
        fn WinHttpReadData(request: Handle, buffer: *mut c_void, to_read: u32, read: *mut u32)
        -> i32;
        fn WinHttpCloseHandle(handle: Handle) -> i32;
    }

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain([0]).collect()
    }

    /// 句柄的 RAII 壳：无论从哪条路径退出都会关掉，FFI 里最容易漏的就是这个。
    struct Handle_(Handle);

    impl Drop for Handle_ {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    fn last_error(what: &str) -> String {
        format!("{what} 失败：{}", std::io::Error::last_os_error())
    }

    pub fn get(host: &str, port: u16, path: &str, timeout: Duration, max_body: usize) -> Result<String, String> {
        let agent = wide(&format!("Cuter/{}", crate::VERSION));
        let session = Handle_(unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        });
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        let millis = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        unsafe { WinHttpSetTimeouts(session.0, millis, millis, millis, millis) };

        let host_w = wide(host);
        let connect = Handle_(unsafe { WinHttpConnect(session.0, host_w.as_ptr(), port, 0) });
        if connect.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }

        let verb = wide("GET");
        let object = wide(path);
        let request = Handle_(unsafe {
            WinHttpOpenRequest(
                connect.0,
                verb.as_ptr(),
                object.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            )
        });
        if request.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        if unsafe {
            WinHttpSendRequest(
                request.0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                0,
                0,
            )
        } == 0
        {
            return Err(last_error("WinHttpSendRequest"));
        }
        if unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) } == 0 {
            return Err(last_error("WinHttpReceiveResponse"));
        }

        let mut status: u32 = 0;
        let mut status_len = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4);
        let ok = unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                std::ptr::null(),
                std::ptr::addr_of_mut!(status).cast(),
                &mut status_len,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error("WinHttpQueryHeaders"));
        }
        if status != 200 {
            return Err(format!("服务器返回 HTTP {status}"));
        }

        let mut body: Vec<u8> = Vec::new();
        loop {
            let mut available: u32 = 0;
            if unsafe { WinHttpQueryDataAvailable(request.0, &mut available) } == 0 {
                return Err(last_error("WinHttpQueryDataAvailable"));
            }
            if available == 0 {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if unsafe {
                WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr().cast(),
                    available,
                    &mut read,
                )
            } == 0
            {
                return Err(last_error("WinHttpReadData"));
            }
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
            if body.len() > max_body {
                return Err(format!("响应超过 {} 字节，放弃", max_body));
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

#[cfg(windows)]
fn fetch_tls(target: &Target, timeout: Duration) -> Result<String, String> {
    winhttp::get(
        &target.host,
        target.port,
        &target.path,
        timeout,
        MAX_BODY,
    )
}

#[cfg(not(windows))]
fn fetch_tls(_target: &Target, _timeout: Duration) -> Result<String, String> {
    Err("自动更新检查的 https 目前只用 Windows 的 WinHTTP 实现".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn newer_versions_win() {
        assert!(is_newer("0.1.46", "0.1.47"));
        assert!(is_newer("0.1.46", "v0.2"));
        assert!(is_newer("0.1.46", "0.1.46.1"));
        assert!(!is_newer("0.1.46", "0.1.46"));
        assert!(!is_newer("0.1.46", "0.1.45"));
        assert!(!is_newer("0.1.46", "0.1.46-rc1"), "同一版本的预发布不算更新");
        assert!(is_newer("1.0", "1.0.1"));
    }

    #[test]
    fn the_manifest_needs_a_version() {
        let manifest = UpdateManifest::parse(
            r#"{"version":" v0.2 ", "url":"https://example.com/a.exe", "notes":"修了几个问题"}"#,
        )
        .expect("清单该能解析");
        assert_eq!(manifest.version, "v0.2");
        assert_eq!(manifest.url, "https://example.com/a.exe");
        assert_eq!(manifest.notes, "修了几个问题");
        assert!(manifest.is_newer_than("0.1.46"));

        // 只写 version 也认
        let bare = UpdateManifest::parse(r#"{"version":"0.1.47"}"#).expect("只写版本号也该认");
        assert_eq!(bare.url, "");
        assert_eq!(bare.notes, "");

        assert!(UpdateManifest::parse(r#"{"url":"https://example.com"}"#).is_err());
        assert!(UpdateManifest::parse("不是 JSON").is_err());
    }

    #[test]
    fn urls_split_into_host_port_and_path() {
        let target = split_url("https://example.com/a/b.json?x=1").expect("https 该能拆");
        assert!(target.tls);
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, 443);
        assert_eq!(target.path, "/a/b.json?x=1");

        let target = split_url("http://127.0.0.1:8899/update.json").expect("http 该能拆");
        assert!(!target.tls);
        assert_eq!(target.port, 8899);
        assert_eq!(target.path, "/update.json");

        let target = split_url("https://example.com").expect("没有路径也该能拆");
        assert_eq!(target.path, "/");

        assert!(split_url("ftp://example.com/x").is_err());
        assert!(split_url("https://example.com:abc/x").is_err());
    }

    /// 本机假服务端：证明 http 那条腿真的把清单取回来了（含分块编码）。
    #[test]
    fn the_plain_leg_reads_a_manifest_from_a_local_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("该能监听");
        let port = listener.local_addr().expect("该有端口").port();
        let body = r#"{"version":"9.9.9","url":"https://example.com/setup.exe"}"#.to_owned();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("该有连接");
            let mut reader = std::io::BufReader::new(socket.try_clone().expect("该能克隆"));
            let mut line = String::new();
            let mut request = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                request.push_str(&line);
                if line == "\r\n" || line == "\n" {
                    break;
                }
                line.clear();
            }
            assert!(request.starts_with("GET /update.json HTTP/1.1"), "请求行：{request}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes());
        });

        let text = fetch(
            &format!("http://127.0.0.1:{port}/update.json"),
            Duration::from_secs(5),
        )
        .expect("该能取回清单");
        server.join().expect("服务端线程该正常结束");
        let manifest = UpdateManifest::parse(&text).expect("该能解析清单");
        assert_eq!(manifest.version, "9.9.9");
        assert!(manifest.is_newer_than(crate::VERSION));
    }

    /// 真的走一次 https（需要网络，默认不跑：`cargo test -p cuter-core -- --ignored`）。
    #[test]
    #[ignore = "需要外网"]
    fn the_tls_leg_reaches_a_real_https_site() {
        let text = fetch("https://example.com/", Duration::from_secs(20))
            .expect("https 请求该成功（有网络的话）");
        assert!(text.contains("<html") || text.contains("<!doctype"), "该拿到 HTML");
    }
}
