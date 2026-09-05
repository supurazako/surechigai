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
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_ATTEMPTS: u32 = 90; // 2秒 x 90 = 最大180秒待つ（画像生成は実測30〜90秒程度かかることがある）
const SUBMIT_RETRY_ATTEMPTS: u32 = 3;

/// 完成した文章の画像生成状況。Web Viewerの表示用にState経由で共有する。
#[derive(Clone, Debug)]
pub struct ImageStatus {
    /// "送信中" / "queued" / "working" / "done" / "error" / "timeout" / "送信失敗" のいずれか
    pub status: String,
    pub image_url: Option<String>,
}

/// 文章の完成ラウンド（`Sentence::round`）ごとに追跡する。
/// 新しいラウンドの追跡が始まったら、古いラウンドの更新は無視して上書き競合を防ぐ。
pub(crate) struct Tracked {
    round: Uuid,
    pub(crate) status: Option<ImageStatus>,
}

pub(crate) type ImageStatusHandle = Arc<Mutex<Option<Tracked>>>;

pub(crate) fn new_image_status_handle() -> ImageStatusHandle {
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
    let status_line = text.lines().next().unwrap_or_default();
    let status_code = status_line.split_whitespace().nth(1);
    if status_code != Some("200") {
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

/// このラウンドの追跡を最新として登録する（他ラウンドの古い更新から保護するため）。
fn start(handle: &ImageStatusHandle, round: Uuid, status: &str) {
    if let Ok(mut guard) = handle.lock() {
        *guard = Some(Tracked {
            round,
            status: Some(ImageStatus {
                status: status.to_string(),
                image_url: None,
            }),
        });
    }
}

/// 現在追跡中のラウンドと一致する場合のみ状態を更新する。
/// 一致しない場合は、より新しいラウンドの追跡がすでに始まっているとみなして何もしない。
fn update(handle: &ImageStatusHandle, round: Uuid, status: &str, image_url: Option<String>) {
    if let Ok(mut guard) = handle.lock() {
        let is_current = guard.as_ref().is_some_and(|tracked| tracked.round == round);
        if is_current {
            *guard = Some(Tracked {
                round,
                status: Some(ImageStatus {
                    status: status.to_string(),
                    image_url,
                }),
            });
        }
    }
}

fn submit_with_retry(submit_url: &ParsedUrl, body: &str) -> std::io::Result<String> {
    let mut last_err = None;
    for attempt in 0..SUBMIT_RETRY_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
        }
        match request(submit_url, "POST", Some(body)) {
            Ok(text) => return Ok(text),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap())
}

fn track_blocking(post_url: &str, device: &str, sentence: &str, round: Uuid, handle: &ImageStatusHandle) {
    let Some(submit_url) = parse_http_url(post_url) else {
        start(handle, round, "post_urlを解釈できません");
        return;
    };
    let origin = origin_of(&submit_url);
    start(handle, round, "送信中");

    let body = json_body(device, sentence);
    let response_text = match submit_with_retry(&submit_url, &body) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("広場サーバへの送信でエラー: {err}");
            update(handle, round, "送信失敗", None);
            return;
        }
    };
    let Some(id) = serde_json::from_str::<Value>(&response_text)
        .ok()
        .and_then(|value| value.get("id").and_then(Value::as_i64))
    else {
        eprintln!("広場サーバの応答からidを取得できません: {response_text}");
        update(handle, round, "送信失敗", None);
        return;
    };
    update(handle, round, "queued", None);

    let Some(job_url) = parse_http_url(&format!("{origin}/jobs/{id}")) else {
        return;
    };
    for _ in 0..POLL_ATTEMPTS {
        std::thread::sleep(POLL_INTERVAL);
        let Ok(text) = request(&job_url, "GET", None) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match status.as_str() {
            "done" => {
                let image_url = value
                    .get("image")
                    .and_then(Value::as_str)
                    .map(|path| format!("{origin}{path}"));
                update(handle, round, "done", image_url);
                return;
            }
            "error" => {
                update(handle, round, "error", None);
                return;
            }
            _ => {
                update(handle, round, &status, None);
            }
        }
    }
    update(handle, round, "timeout", None);
}

/// 完成した文章を広場サーバへ非同期にPOSTし、画像生成の完了までポーリングして
/// `handle` に反映する。エラーはログに出すだけで交換処理は止めない。
/// `round` は`Sentence::round`。同時に複数ラウンドが追跡されても、
/// 最後に`spawn_post`されたラウンドの更新だけがWeb Viewerに反映される。
pub(crate) fn spawn_post(
    post_url: String,
    device: String,
    sentence: String,
    round: Uuid,
    handle: ImageStatusHandle,
) {
    tokio::task::spawn_blocking(move || {
        track_blocking(&post_url, &device, &sentence, round, &handle);
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
    fn reads_status_and_image_from_job_response() {
        let text = r#"{"id":1,"device":"A","sentence":"...","status":"done","image":"/image/1.jpg","error":null,"created":1.0}"#;
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(value.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            value.get("image").and_then(Value::as_str),
            Some("/image/1.jpg")
        );
    }

    #[test]
    fn stale_round_update_is_ignored_after_newer_round_starts() {
        let handle = new_image_status_handle();
        let old_round = Uuid::new_v4();
        let new_round = Uuid::new_v4();

        start(&handle, old_round, "送信中");
        update(&handle, old_round, "queued", None);
        // 新しいラウンドが追跡を開始した後は、古いラウンドの更新は無視される。
        start(&handle, new_round, "送信中");
        update(&handle, old_round, "done", Some("http://x/image/1.jpg".into()));

        let guard = handle.lock().unwrap();
        let tracked = guard.as_ref().unwrap();
        assert_eq!(tracked.round, new_round);
        assert_eq!(tracked.status.as_ref().unwrap().status, "送信中");
    }
}
