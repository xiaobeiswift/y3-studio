use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

pub mod session;

/// 4字节大端长度头 + UTF-8 JSON 协议编码器与粘包/半包解码器
pub struct ConsoleProtocol {
    buffer: Vec<u8>,
}

impl Default for ConsoleProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleProtocol {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn encode(obj: &Value) -> Vec<u8> {
        let content = serde_json::to_vec(obj).unwrap_or_default();
        let len = content.len() as u32;
        let mut out = Vec::with_capacity(4 + content.len());
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&content);
        out
    }

    pub fn feed_and_decode(&mut self, data: &[u8]) -> Vec<Value> {
        self.buffer.extend_from_slice(data);
        let mut messages = Vec::new();
        while self.buffer.len() >= 4 {
            let len = u32::from_be_bytes([
                self.buffer[0],
                self.buffer[1],
                self.buffer[2],
                self.buffer[3],
            ]) as usize;
            if self.buffer.len() < 4 + len {
                break;
            }
            let payload: Vec<u8> = self.buffer.drain(0..4 + len).skip(4).collect();
            if let Ok(val) = serde_json::from_slice::<Value>(&payload) {
                messages.push(val);
            }
        }
        messages
    }
}

/// 单个日志文件管理器（支持自动清理超出上限的旧日志，与原版 `LogManager` 一致）
pub struct LogManager {
    pub log_file_path: PathBuf,
    lines: Mutex<Vec<String>>,
}

impl LogManager {
    const MAX_LOG_FILES: usize = 20;
    const MAX_MEMORY_LINES: usize = 10000;

    pub fn new(prefix: &str) -> Self {
        let log_dir = std::env::temp_dir().join("y3-helper-logs");
        let _ = fs::create_dir_all(&log_dir);
        Self::cleanup_old_logs(&log_dir);

        let ts = chrono::Utc::now().timestamp_millis();
        let safe_prefix: String = prefix
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let log_file_path = log_dir.join(format!("session-{}-{}.log", safe_prefix, ts));
        let _ = fs::File::create(&log_file_path);

        Self {
            log_file_path,
            lines: Mutex::new(Vec::new()),
        }
    }

    fn cleanup_old_logs(log_dir: &Path) {
        if let Ok(entries) = fs::read_dir(log_dir) {
            let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("log") {
                        let mtime = e.metadata().ok()?.modified().ok()?;
                        Some((p, mtime))
                    } else {
                        None
                    }
                })
                .collect();
            files.sort_by(|a, b| b.1.cmp(&a.1));
            if files.len() >= Self::MAX_LOG_FILES {
                for (old_path, _) in files.into_iter().skip(Self::MAX_LOG_FILES - 1) {
                    let _ = fs::remove_file(old_path);
                }
            }
        }
    }

    pub fn append_log(&self, message: &str) {
        let ts = chrono::Utc::now().to_rfc3339();
        let formatted = format!("[{}] {}", ts, message);
        {
            let mut guard = self.lines.lock();
            for line in message.lines() {
                if !line.trim().is_empty() {
                    guard.push( format!("[{}] {}", ts, line) );
                }
            }
            if guard.len() > Self::MAX_MEMORY_LINES {
                let excess = guard.len() - Self::MAX_MEMORY_LINES;
                guard.drain(0..excess);
            }
        }
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file_path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{}", formatted);
        }
    }

    pub fn read_logs(&self, limit: usize) -> Vec<String> {
        let guard = self.lines.lock();
        let len = guard.len();
        let start = len.saturating_sub(limit);
        guard[start..].to_vec()
    }

    pub fn total_lines(&self) -> usize {
        self.lines.lock().len()
    }
}

/// 游戏客户端自定义树节点（内存中保留供查询，替代 VS Code TreeView）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTreeInfo {
    pub id: u64,
    pub name: String,
    pub root: u64,
}

/// 单个已连接的游戏窗口客户端（对应原版 `Client`）
pub struct ConsoleClient {
    pub id: u64,
    /// 窗口稳定槽位号（0, 1, 2...，断开后自动释放可复用）
    pub slot: usize,
    name: RwLock<String>,
    multi_mode: AtomicBool,
    closed: AtomicBool,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    next_req_id: AtomicU64,
    pending_requests: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    pub log_manager: Arc<LogManager>,
    pub custom_trees: RwLock<HashMap<u64, CustomTreeInfo>>,
}

impl ConsoleClient {
    pub fn name(&self) -> String {
        self.name.read().clone()
    }

    pub fn set_name(&self, new_name: String) {
        *self.name.write() = new_name;
    }

    pub fn is_multi_mode(&self) -> bool {
        self.multi_mode.load(Ordering::Relaxed)
    }

    pub fn set_multi_mode(&self, multi: bool) {
        self.multi_mode.store(multi, Ordering::Relaxed);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// 发送单向通知（如 `notify("command", json!({"data": ".rd"}))`）
    pub fn notify(&self, method: &str, params: Value) {
        if self.is_closed() {
            return;
        }
        let msg = json!({
            "method": method,
            "params": params,
        });
        let encoded = ConsoleProtocol::encode(&msg);
        let _ = self.tx.send(encoded);
    }

    /// 发送双向请求并等待游戏客户端响应
    pub async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        if self.is_closed() {
            anyhow::bail!("客户端已断开连接");
        }
        let req_id = self.next_req_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending_requests.lock().insert(req_id, tx);

        let msg = json!({
            "method": method,
            "id": req_id,
            "params": params,
        });
        let encoded = ConsoleProtocol::encode(&msg);
        self.tx
            .send(encoded)
            .map_err(|_| anyhow::anyhow!("发送请求失败：通道已关闭"))?;

        match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
            Ok(Ok(Ok(val))) => Ok(val),
            Ok(Ok(Err(err_msg))) => anyhow::bail!("客户端返回错误: {}", err_msg),
            Ok(Err(_)) => anyhow::bail!("请求被取消（客户端已断开）"),
            Err(_) => {
                self.pending_requests.lock().remove(&req_id);
                anyhow::bail!("等待客户端响应超时")
            }
        }
    }

    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let mut pending = self.pending_requests.lock();
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err("客户端连接已关闭".to_string()));
            }
        }
    }

    fn send_raw_json(&self, obj: &Value) {
        if !self.is_closed() {
            let encoded = ConsoleProtocol::encode(obj);
            let _ = self.tx.send(encoded);
        }
    }

    fn handle_incoming(&self, obj: Value) {
        if let Some(method) = obj.get("method").and_then(|v| v.as_str()) {
            let params = obj.get("params").cloned().unwrap_or(Value::Null);
            let req_id = obj.get("id").and_then(|v| v.as_u64());

            match method {
                "print" => {
                    if let Some(msg) = params.get("message").and_then(|v| v.as_str()) {
                        self.log_manager.append_log(msg);
                    }
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({ "id": id, "result": true }));
                    }
                }
                "updatePlayer" => {
                    if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
                        self.set_name(name.to_string());
                    }
                    if let Some(multi) = params.get("multiMode").and_then(|v| v.as_bool()) {
                        self.set_multi_mode(multi);
                    }
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({ "id": id, "result": true }));
                    }
                }
                "prepareForRestart" => {
                    self.log_manager.append_log("[Y3-Helper] 准备重启游戏...");
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({ "id": id, "result": true }));
                    }
                }
                "createTreeView" => {
                    if let (Some(tree_id), Some(name), Some(root)) = (
                        params.get("id").and_then(|v| v.as_u64()),
                        params.get("name").and_then(|v| v.as_str()),
                        params.get("root").and_then(|v| v.as_u64()),
                    ) {
                        self.custom_trees.write().insert(
                            tree_id,
                            CustomTreeInfo {
                                id: tree_id,
                                name: name.to_string(),
                                root,
                            },
                        );
                    }
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({ "id": id, "result": true }));
                    }
                }
                "removeTreeView" => {
                    if let Some(tree_id) = params.get("id").and_then(|v| v.as_u64()) {
                        self.custom_trees.write().remove(&tree_id);
                    }
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({ "id": id, "result": true }));
                    }
                }
                "refreshTreeNode" | "createTracy" | "command" => {
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({ "id": id, "result": true }));
                    }
                }
                _ => {
                    if let Some(id) = req_id {
                        self.send_raw_json(&json!({
                            "id": id,
                            "error": format!("未找到方法\"{}\"", method)
                        }));
                    }
                }
            }
        } else if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
            if let Some(sender) = self.pending_requests.lock().remove(&id) {
                if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
                    let _ = sender.send(Err(err.to_string()));
                } else {
                    let res = obj.get("result").cloned().unwrap_or(Value::Null);
                    let _ = sender.send(Ok(res));
                }
            }
        }
    }
}

/// 每个项目独立的 TCP 控制台服务器（支持多项目同时监听各自端口并管理各自的游戏窗口）
pub struct ConsoleServer {
    pub project_id: String,
    pub port: u16,
    clients: Arc<RwLock<Vec<Arc<ConsoleClient>>>>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl ConsoleServer {
    /// 启动一个新的 `ConsoleServer`（若 `preferred_port` 为 0 或已被占用，则自动分配空闲端口）
    pub async fn start(project_id: &str, preferred_port: u16) -> anyhow::Result<Arc<Self>> {
        let listener = match TcpListener::bind(("127.0.0.1", preferred_port)).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let port = listener.local_addr()?.port();
        let clients: Arc<RwLock<Vec<Arc<ConsoleClient>>>> = Arc::new(RwLock::new(Vec::new()));
        let next_client_id = Arc::new(AtomicU64::new(1));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let server = Arc::new(Self {
            project_id: project_id.to_string(),
            port,
            clients: clients.clone(),
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
        });

        let proj_id_owned = project_id.to_string();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((stream, _addr)) => {
                                Self::handle_connection(
                                    stream,
                                    &proj_id_owned,
                                    clients.clone(),
                                    next_client_id.clone(),
                                );
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok(server)
    }

    fn handle_connection(
        stream: TcpStream,
        project_id: &str,
        clients: Arc<RwLock<Vec<Arc<ConsoleClient>>>>,
        next_client_id: Arc<AtomicU64>,
    ) {
        let (mut reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

        let client_id = next_client_id.fetch_add(1, Ordering::Relaxed);
        let client = {
            let mut list = clients.write();
            let mut slot = 0usize;
            while list.iter().any(|c| c.slot == slot) {
                slot += 1;
            }
            let log_manager = Arc::new(LogManager::new(&format!("{}_slot{}", project_id, slot)));
            let c = Arc::new(ConsoleClient {
                id: client_id,
                slot,
                name: RwLock::new("默认客户端".to_string()),
                multi_mode: AtomicBool::new(false),
                closed: AtomicBool::new(false),
                tx,
                next_req_id: AtomicU64::new(1),
                pending_requests: Mutex::new(HashMap::new()),
                log_manager,
                custom_trees: RwLock::new(HashMap::new()),
            });
            list.push(c.clone());
            c
        };

        // 写协程
        tokio::spawn(async move {
            while let Some(packet) = rx.recv().await {
                if writer.write_all(&packet).await.is_err() {
                    break;
                }
            }
        });

        // 读协程
        let client_for_read = client.clone();
        let clients_for_cleanup = clients.clone();
        tokio::spawn(async move {
            let mut protocol = ConsoleProtocol::new();
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        for msg in protocol.feed_and_decode(&buf[..n]) {
                            client_for_read.handle_incoming(msg);
                        }
                    }
                }
            }
            client_for_read.close();
            clients_for_cleanup
                .write()
                .retain(|c| c.id != client_for_read.id);
        });
    }

    /// 获取当前项目已连接的所有客户端列表
    pub fn list_clients(&self) -> Vec<Arc<ConsoleClient>> {
        self.clients.read().clone()
    }

    /// 按稳定窗口槽位号 `slot` 查找客户端
    pub fn find_client_by_slot(&self, slot: usize) -> Option<Arc<ConsoleClient>> {
        self.clients.read().iter().find(|c| c.slot == slot).cloned()
    }

    /// 向该项目所有已连接的游戏客户端发送 `.rd` 热重载指令
    pub fn reload_lua_all(&self) -> usize {
        let list = self.list_clients();
        for c in &list {
            c.notify("command", json!({ "data": ".rd" }));
        }
        list.len()
    }

    /// 将当前 `ConsoleServer` 的端口号写入指定脚本目录下的 `log/helper_port.lua`
    pub fn write_helper_port_file(&self, script_dir: &Path) -> anyhow::Result<PathBuf> {
        let log_dir = script_dir.join("log");
        fs::create_dir_all(&log_dir)?;
        let file_path = log_dir.join("helper_port.lua");
        fs::write(&file_path, format!("return {}", self.port))?;
        Ok(file_path)
    }

    pub fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().take() {
            let _ = tx.send(());
        }
        for c in self.list_clients() {
            c.close();
        }
        self.clients.write().clear();
    }
}

impl Drop for ConsoleServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_console_server_and_protocol() {
        let server = ConsoleServer::start("test_proj", 0).await.unwrap();
        assert!(server.port > 0);

        // 模拟游戏客户端连接
        let mut stream = TcpStream::connect(("127.0.0.1", server.port))
            .await
            .unwrap();

        // 发送 updatePlayer 与 print
        let msg1 = ConsoleProtocol::encode(&json!({
            "method": "updatePlayer",
            "params": { "name": "玩家1", "multiMode": true }
        }));
        let msg2 = ConsoleProtocol::encode(&json!({
            "method": "print",
            "params": { "message": "Hello from Y3 Game!" }
        }));
        stream.write_all(&msg1).await.unwrap();
        stream.write_all(&msg2).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let clients = server.list_clients();
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].slot, 0);
        assert_eq!(clients[0].name(), "玩家1");
        assert!(clients[0].is_multi_mode());

        let logs = clients[0].log_manager.read_logs(10);
        assert_eq!(logs.len(), 1);
        assert!(logs[0].contains("Hello from Y3 Game!"));
    }
}
