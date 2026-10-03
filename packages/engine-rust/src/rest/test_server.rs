//! A tiny loopback HTTP server for tests: it answers every request with a JSON echo of what it received,
//! except for paths a test registered a fixed answer for.

use std::{collections::HashMap, sync::Arc};

use serde_json::{Map, Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// A fixed answer: content type and body.
pub type Canned = HashMap<String, (String, String)>;

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Start the server; returns its base URL (`http://127.0.0.1:port`). `canned` maps a path to the answer for it.
pub async fn start(canned: Canned) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a loopback port");
    let port = listener.local_addr().expect("a local address").port();
    let canned = Arc::new(canned);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let canned = canned.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    if let Some(at) = find(&data, b"\r\n\r\n") {
                        break at;
                    }
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => data.extend_from_slice(&chunk[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
                let mut lines = head.split("\r\n");
                let mut first = lines.next().unwrap_or("").split(' ');
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                let mut headers = Map::new();
                for line in lines {
                    if let Some((name, value)) = line.split_once(':') {
                        headers.insert(name.trim().to_ascii_lowercase(), json!(value.trim()));
                    }
                }
                let length: usize =
                    headers.get("content-length").and_then(Value::as_str).and_then(|v| v.parse().ok()).unwrap_or(0);
                while data.len() < head_end + 4 + length {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => data.extend_from_slice(&chunk[..n]),
                    }
                }
                let body = String::from_utf8_lossy(&data[head_end + 4..]).into_owned();
                let route = path.split('?').next().unwrap_or("");
                let (content_type, answer) = match canned.get(route) {
                    Some((content_type, body)) => (content_type.clone(), body.clone()),
                    None => (
                        "application/json".to_string(),
                        json!({ "method": method, "path": path, "headers": headers, "body": body }).to_string(),
                    ),
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}
