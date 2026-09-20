//! Preserve late ingress requests while a retired Server releases its Nodes.
use axum::{body::Body, extract::Request, http::HeaderMap, response::Response};
use std::{
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
};

// reqwest requires a Sync body. Polling remains synchronous and never holds
// this mutex across an await; frames (including trailers) pass through intact.
struct RequestBody(Mutex<Body>);
impl http_body::Body for RequestBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut *self.0.lock().unwrap()).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.0.lock().unwrap().is_end_stream()
    }
    fn size_hint(&self) -> http_body::SizeHint {
        self.0.lock().unwrap().size_hint()
    }
}

fn strip_connection_headers(headers: &mut HeaderMap) {
    let nominated: Vec<String> = headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(',').map(|name| name.trim().to_owned()))
        .collect();
    for name in nominated {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

pub fn local(path: &str) -> bool {
    matches!(
        path,
        "/api/admin/release" | "/api/admin/release/retire" | "/api/nodes/channel"
    )
}

pub async fn forward(port: u16, request: Request) -> anyhow::Result<Response> {
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", |value| value.as_str());
    let url = format!("http://127.0.0.1:{port}{path}");
    let (mut parts, body) = request.into_parts();
    strip_connection_headers(&mut parts.headers);
    // A successor rechecks the original caller's identity. Never substitute
    // the service/admin credential or follow a redirect carrying credentials.
    let response = reqwest::Client::builder()
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()?
        .request(parts.method, url)
        .headers(parts.headers)
        .header("te", "trailers")
        .body(reqwest::Body::wrap(RequestBody(Mutex::new(body))))
        .send()
        .await?;
    let response: axum::http::Response<reqwest::Body> = response.into();
    let (mut parts, body) = response.into_parts();
    strip_connection_headers(&mut parts.headers);
    Ok(Response::from_parts(parts, Body::new(body)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body::Body as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Frames(std::collections::VecDeque<http_body::Frame<axum::body::Bytes>>);
    impl http_body::Body for Frames {
        type Data = axum::body::Bytes;
        type Error = std::convert::Infallible;
        fn poll_frame(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            Poll::Ready(self.0.pop_front().map(Ok))
        }
    }

    #[tokio::test]
    async fn relay_preserves_caller_query_stream_frames_and_trailers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0; 1];
            while !request.ends_with(b"x-upload: complete\r\n\r\n") {
                assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap().to_lowercase();
            for expected in [
                "post /api/executions?q=a%2fb http/1.1",
                "authorization: bearer caller",
                "cookie: identity=caller",
                "last-event-id: 17",
                "range: bytes=1-5",
                "te: trailers",
                "payload",
            ] {
                assert!(request.contains(expected), "missing {expected}: {request}");
            }
            assert!(!request.contains("x-hop:"));
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nTransfer-Encoding: chunked\r\nTrailer: x-result\r\nContent-Range: bytes 1-5/10\r\nSet-Cookie: one=1\r\nSet-Cookie: two=2\r\nConnection: close, x-hop\r\nX-Hop: private\r\n\r\n5\r\nhello\r\n0\r\nx-result: durable\r\n\r\n").await.unwrap();
        });
        let mut trailers = HeaderMap::new();
        trailers.insert("x-upload", "complete".parse().unwrap());
        let request = Request::builder()
            .method("POST")
            .uri("/api/executions?q=a%2Fb")
            .header("authorization", "Bearer caller")
            .header("cookie", "identity=caller")
            .header("last-event-id", "17")
            .header("range", "bytes=1-5")
            .header("connection", "keep-alive, x-hop")
            .header("x-hop", "private")
            .header("trailer", "x-upload")
            .body(Body::new(Frames(std::collections::VecDeque::from([
                http_body::Frame::data("payload".into()),
                http_body::Frame::trailers(trailers),
            ]))))
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let response = forward(port, request).await.unwrap();
            assert_eq!(response.status(), 206);
            assert_eq!(response.headers()["content-range"], "bytes 1-5/10");
            assert_eq!(response.headers().get_all("set-cookie").iter().count(), 2);
            assert!(!response.headers().contains_key("x-hop"));
            let mut body = response.into_body();
            let mut bytes = Vec::new();
            let mut result = None;
            while let Some(frame) =
                futures::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
            {
                let frame = frame.unwrap();
                if let Some(data) = frame.data_ref() {
                    bytes.extend_from_slice(data);
                }
                if let Some(trailers) = frame.trailers_ref() {
                    result = trailers.get("x-result").cloned();
                }
            }
            assert_eq!(bytes, b"hello");
            assert_eq!(result.unwrap(), "durable");
            peer.await.unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn relay_returns_redirect_and_encoded_body_without_following_or_decoding() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0; 1];
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                request.push(byte[0]);
            }
            socket.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nContent-Encoding: gzip\r\nContent-Length: 5\r\nLocation: http://127.0.0.1:1/private\r\nConnection: close\r\n\r\nbytes").await.unwrap();
        });
        let request = Request::builder()
            .uri("/download")
            .body(Body::empty())
            .unwrap();
        let response = forward(port, request).await.unwrap();
        assert_eq!(response.status(), 307);
        assert_eq!(response.headers()["content-encoding"], "gzip");
        assert_eq!(response.headers()["location"], "http://127.0.0.1:1/private");
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "bytes"
        );
        peer.await.unwrap();
    }
}
