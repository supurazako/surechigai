//! 完成した文章を広場サーバ（`server/`）へ通知し、生成された画像を追跡する
//! 最小限のHTTPクライアント。std::netで素朴に実装し、JSON解析のみserde_jsonを使う。
//! TLSはサポートしない（`http://`のみ）。失敗しても交換自体は継続する。

use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex},
    time::Duration,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_ATTEMPTS: u32 = 40; // 2秒 x 40 = 最大80秒待つ

/// 完成した文章の画像生成状況。Web Viewerの表示用にState経由で共有する。
#[derive(Clone, Debug)]
pub struct ImageStatus {
    /// "queued" / "working" / "done" / "error" / "timeout" / "送信失敗" のいずれか
    pub status: String,
    pub image_url: Option<String>,
}

pub type ImageStatusHandle = Arc<Mutex<Option<ImageStatus>>>;

pub fn new_image_status_handle() -> ImageStatusHandle {
    Arc::new(Mutex::new(None))
}

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Option<ParsedUrl> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().ok()?),
        None => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return None;
    }
    Some(ParsedUrl {
        host,
        port,
        path: path.to_string(),
    })
}

fn origin_of(parsed: &ParsedUrl) -> String {
    format!("http://{}:{}", parsed.host, parsed.port)
}

/// `{"device":"<device>","sentence":"<sentence>"}` をJSONエスケープして組み立てる。
fn json_body(device: &str, sentence: &str) -> String {
    fn escape(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for c in value.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out
    }
    format!(
        "{{\"device\":\"{}\",\"sentence\":\"{}\"}}",
        escape(device),
        escape(sentence)
    )
}

/// 1回のHTTP/1.1リクエストを送り、レスポンスボディ（ヘッダーを除く）を返す。
fn request(parsed: &ParsedUrl, method: &str, body: Option<&str>) -> std::io::Result<String> {
    let request_text = match body {
        Some(body) => format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            method = method,
            path = parsed.path,
            host = parsed.host,
            len = body.len(),
            body = body,
        ),
        None => format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Connection: close\r\n\
             \r\n",
            method = method,
            path = parsed.path,
            host = parsed.host,
        ),
    };

    let addr = (parsed.host.as_str(), parsed.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "ホスト名を解決できません")
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.write_all(request_text.as_bytes())?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;

    let text = String::from_utf8_lossy(&response);
    let status_line = text.lines().next().unwrap_or_default().to_string();
    if !status_line.contains("200") {
        return Err(std::io::Error::other(format!(
            "HTTPステータス異常: {status_line}"
        )));
    }
    let body_text = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or("");
    Ok(body_text.to_string())
}

fn update(handle: &ImageStatusHandle, status: &str, image_url: Option<String>) {
    if let Ok(mut guard) = handle.lock() {
        *guard = Some(ImageStatus {
            status: status.to_string(),
            image_url,
        });
    }
}

fn track_blocking(post_url: &str, device: &str, sentence: &str, handle: &ImageStatusHandle) {
    let Some(submit_url) = parse_http_url(post_url) else {
        update(handle, "post_urlを解釈できません", None);
        return;
    };
    let origin = origin_of(&submit_url);

    update(handle, "送信中", None);
    let body = json_body(device, sentence);
    let response_text = match request(&submit_url, "POST", Some(&body)) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("広場サーバへの送信でエラー: {err}");
            update(handle, "送信失敗", None);
            return;
        }
    };
    let Some(id) = serde_json::from_str::<Value>(&response_text)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_i64))
    else {
        eprintln!("広場サーバの応答からidを取得できません: {response_text}");
        update(handle, "送信失敗", None);
        return;
    };
    update(handle, "queued", None);

    let Some(latest_url) = parse_http_url(&format!("{origin}/latest.json")) else {
        return;
    };
    for _ in 0..POLL_ATTEMPTS {
        std::thread::sleep(POLL_INTERVAL);
        let Ok(text) = request(&latest_url, "GET", None) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(items) = value.get("items").and_then(Value::as_array) else {
            continue;
        };
        let Some(item) = items
            .iter()
            .find(|item| item.get("id").and_then(Value::as_i64) == Some(id))
        else {
            continue;
        };
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match status.as_str() {
            "done" => {
                let image_url = item
                    .get("image")
                    .and_then(Value::as_str)
                    .map(|path| format!("{origin}{path}"));
                update(handle, "done", image_url);
                return;
            }
            "error" => {
                update(handle, "error", None);
                return;
            }
            _ => {
                update(handle, &status, None);
            }
        }
    }
    update(handle, "timeout", None);
}

/// 完成した文章を広場サーバへ非同期にPOSTし、画像生成の完了までポーリングして
/// `handle` に反映する。エラーはログに出すだけで交換処理は止めない。
pub fn spawn_post(post_url: String, device: String, sentence: String, handle: ImageStatusHandle) {
    update(&handle, "送信中", None);
    tokio::task::spawn_blocking(move || {
        track_blocking(&post_url, &device, &sentence, &handle);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_port_and_path() {
        let parsed = parse_http_url("http://192.168.1.5:8000/submit").unwrap();
        assert_eq!(parsed.host, "192.168.1.5");
        assert_eq!(parsed.port, 8000);
        assert_eq!(parsed.path, "/submit");
        assert_eq!(origin_of(&parsed), "http://192.168.1.5:8000");
    }

    #[test]
    fn defaults_to_port_80_and_root_path() {
        let parsed = parse_http_url("http://example.com").unwrap();
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 80);
        assert_eq!(parsed.path, "/");
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(parse_http_url("https://example.com/submit").is_none());
        assert!(parse_http_url("not-a-url").is_none());
    }

    #[test]
    fn escapes_json_body() {
        let body = json_body("A", "暇だったので\n\"すごい\"");
        assert_eq!(
            body,
            "{\"device\":\"A\",\"sentence\":\"暇だったので\\n\\\"すごい\\\"\"}"
        );
    }

    #[test]
    fn extracts_id_from_submit_response() {
        let value: Value = serde_json::from_str(r#"{"id":12,"status":"queued"}"#).unwrap();
        assert_eq!(value.get("id").and_then(Value::as_i64), Some(12));
    }

    #[test]
    fn finds_matching_item_and_reads_image() {
        let text = r#"{"latest_id":2,"items":[
            {"id":2,"device":"B","sentence":"...","status":"working","image":null},
            {"id":1,"device":"A","sentence":"...","status":"done","image":"/image/1.jpg"}
        ]}"#;
        let value: Value = serde_json::from_str(text).unwrap();
        let items = value.get("items").and_then(Value::as_array).unwrap();
        let item = items
            .iter()
            .find(|item| item.get("id").and_then(Value::as_i64) == Some(1))
            .unwrap();
        assert_eq!(item.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            item.get("image").and_then(Value::as_str),
            Some("/image/1.jpg")
        );
    }
}
