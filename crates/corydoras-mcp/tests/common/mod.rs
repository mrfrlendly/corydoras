//! The MCP client that the integration tests share: it starts the server and speaks JSON-RPC over stdio.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

pub struct Client {
    process: Child,
    replies: BufReader<ChildStdout>,
    id: u32,
}

impl Client {
    pub fn connect(path: &Path) -> (Self, String) {
        Self::start(
            Command::new(env!("CARGO_BIN_EXE_corydoras-mcp"))
                .arg("--library")
                .arg(path)
                .env("CORYDORAS_CONFIG_DIR", path),
        )
    }

    pub fn start(command: &mut Command) -> (Self, String) {
        let mut process = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let replies = BufReader::new(process.stdout.take().unwrap());
        let mut client = Self {
            process,
            replies,
            id: 0,
        };
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "test", "version": "1"}
            }),
        );
        let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        writeln!(client.process.stdin.as_mut().unwrap(), "{notification}").unwrap();
        (client, init["instructions"].as_str().unwrap().to_owned())
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let message = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        let pipe = self.process.stdin.as_mut().unwrap();
        writeln!(pipe, "{message}").unwrap();
        pipe.flush().unwrap();
        let mut line = String::new();
        self.replies.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.id);
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    pub fn call(&mut self, name: &str, args: Value) -> Vec<Value> {
        let result = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(result["isError"], false, "{result}");
        result["content"].as_array().unwrap().clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        drop(self.process.stdin.take());
        self.process.wait().unwrap();
    }
}
