use std::path::Path;

use anyhow::Result;

use crate::protocol::{Request, Response};

pub struct DbClient {
    socket_path: std::path::PathBuf,
}

impl DbClient {
    pub async fn connect(socket_path: impl AsRef<Path>) -> Result<Self> {
        // TODO: use `_socket_path` to connect to the daemon socket, spawning
        // forge-dbd if necessary, and store it in the returned DbClient.
        let _socket_path = socket_path.as_ref().to_path_buf();
        todo!("connect to the daemon socket, spawning forge-dbd if necessary");
    }

    pub async fn send(&self, request: Request) -> Result<Response> {
        let _ = request;
        let _ = &self.socket_path;
        todo!("serialize request, write a length-prefixed frame, and await the response");
    }
}
