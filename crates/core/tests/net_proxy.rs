use opencoder_core::net::build_http_client_with_proxy_rules;

#[tokio::test]
async fn configured_exclusion_bypasses_proxy_and_other_hosts_use_it() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    fn serve(listener: TcpListener, body: &'static str) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut socket = loop {
                if let Ok((socket, _)) = listener.accept() {
                    break socket;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "HTTP request did not arrive"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        })
    }
    let direct = TcpListener::bind("127.0.0.2:0").unwrap();
    let target = format!("http://{}", direct.local_addr().unwrap());
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let direct_server = serve(direct, "direct");
    let proxy_server = serve(proxy, "proxy");
    let timeout = std::time::Duration::from_secs(2);
    let excluded =
        build_http_client_with_proxy_rules(Some(&proxy_url), Some("127.0.0.2"), timeout).unwrap();
    assert_eq!(
        excluded
            .get(&target)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "direct"
    );
    let proxied = build_http_client_with_proxy_rules(Some(&proxy_url), None, timeout).unwrap();
    assert_eq!(
        proxied
            .get(&target)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "proxy"
    );
    direct_server.join().unwrap();
    proxy_server.join().unwrap();
}
