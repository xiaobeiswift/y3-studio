use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Y3 网络功能测试服务器（对应原版 `src/networkServer.ts`）：
/// - `127.0.0.1:25895`: 回显服务（收到 `"再见！"` 时回复 `"马达捏~"` 并断开）
/// - `127.0.0.1:25896`: 每秒发送 1 个 4B 大端长度头 + `{count, time}` JSON 包，共 10 次后断开
pub struct NetworkTestServer;

impl NetworkTestServer {
    pub async fn start_both() -> anyhow::Result<(u16, u16)> {
        let listener1 = TcpListener::bind(("127.0.0.1", 25895)).await?;
        let port1 = listener1.local_addr()?.port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener1.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = socket.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let s = String::from_utf8_lossy(&buf[..n]);
                        if s == "再见！" {
                            let _ = socket.write_all("马达捏~".as_bytes()).await;
                            break;
                        } else {
                            let reply = format!("你发送了：{}", s);
                            let _ = socket.write_all(reply.as_bytes()).await;
                        }
                    }
                });
            }
        });

        let listener2 = TcpListener::bind(("127.0.0.1", 25896)).await?;
        let port2 = listener2.local_addr()?.port();

        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener2.accept().await {
                tokio::spawn(async move {
                    for count in 1..=10 {
                        let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                        let body = json!({
                            "count": count,
                            "time": now_str,
                        })
                        .to_string();
                        let len = body.len() as u32;
                        let mut packet = Vec::with_capacity(4 + body.len());
                        packet.extend_from_slice(&len.to_be_bytes());
                        packet.extend_from_slice(body.as_bytes());
                        if socket.write_all(&packet).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                });
            }
        });

        Ok((port1, port2))
    }
}
