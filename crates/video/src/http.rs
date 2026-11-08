//! 同じPCの中のメディアサーバーとやりとりするための、小さな HTTP（std だけ。HTTP/1.0 で1回ずつつなぐ）。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// `http://host:port/path` を (host:port, /path) に分ける
pub fn split_url(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("http:// で始まる URL にしてください（{url}）"))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };
    Ok((host, path.to_string()))
}

fn connect(host: &str, timeout: Duration) -> Result<TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addr = host
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host} が見つかりません"))?;
    let s = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("{host} につながりません: {e}"))?;
    let _ = s.set_nodelay(true);
    Ok(s)
}

/// 応答の頭を読んで、状態コードを返す（本文の読み手を返す）
fn read_head(s: TcpStream) -> Result<(u16, BufReader<TcpStream>), String> {
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    let code: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| format!("応答を読めません: {}", line.trim()))?;
    loop {
        line.clear();
        let n = r.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    Ok((code, r))
}

fn request(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<(u16, BufReader<TcpStream>), String> {
    let (host, path) = split_url(url)?;
    let mut s = connect(&host, timeout)?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    let mut head = format!("{method} {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(b) = body {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
    if let Some(b) = body {
        s.write_all(b).map_err(|e| e.to_string())?;
    }
    read_head(s)
}

/// GET して本文を全部読む（2xx 以外はエラー）
pub fn get(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let (code, mut r) = request("GET", url, None, timeout)?;
    let mut buf = vec![];
    r.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if !(200..300).contains(&code) {
        return Err(format!(
            "HTTP {code}: {}",
            String::from_utf8_lossy(&buf).trim()
        ));
    }
    Ok(buf)
}

/// PUT（JSON）して本文を全部読む
pub fn put_json(url: &str, body: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
    let (code, mut r) = request("PUT", url, Some(body), timeout)?;
    let mut buf = vec![];
    r.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if !(200..300).contains(&code) {
        return Err(format!(
            "HTTP {code}: {}",
            String::from_utf8_lossy(&buf).trim()
        ));
    }
    Ok(buf)
}

/// GET して、本文を読み続ける読み手を返す（MJPEG の流れなど）。読み取りの待ち時間は read_timeout
pub fn stream(url: &str, read_timeout: Duration) -> Result<BufReader<TcpStream>, String> {
    let (code, r) = request("GET", url, None, Duration::from_secs(3))?;
    if !(200..300).contains(&code) {
        return Err(format!("HTTP {code}"));
    }
    r.get_ref().set_read_timeout(Some(read_timeout)).ok();
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            split_url("http://127.0.0.1:47810/region/face-A.mjpg").unwrap(),
            ("127.0.0.1:47810".into(), "/region/face-A.mjpg".into())
        );
        assert_eq!(
            split_url("http://example").unwrap(),
            ("example:80".into(), "/".into())
        );
        assert!(split_url("rtmp://x").is_err());
    }
}
