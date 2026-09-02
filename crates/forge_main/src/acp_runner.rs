use std::future::Future;

use anyhow::Result;
use forge_api::API;

/// Abstraction over the ACP stdio transport entry point for testability.
pub trait MachineStdioApi {
    fn acp_start_stdio(&self) -> impl Future<Output = Result<()>> + Send;
}

impl<T: API + Sync> MachineStdioApi for T {
    fn acp_start_stdio(&self) -> impl Future<Output = Result<()>> + Send {
        API::acp_start_stdio(self)
    }
}

/// Starts the ACP machine stdio server by delegating to the provided API.
pub async fn run_machine_stdio_server<A: MachineStdioApi + ?Sized>(api: &A) -> Result<()> {
    api.acp_start_stdio().await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    use anyhow::Result;

    use super::{MachineStdioApi, run_machine_stdio_server};

    struct MockApi {
        called: Arc<AtomicBool>,
    }

    impl MockApi {
        fn new(called: Arc<AtomicBool>) -> Self {
            Self { called }
        }
    }

    impl MachineStdioApi for MockApi {
        fn acp_start_stdio(&self) -> impl std::future::Future<Output = Result<()>> + Send {
            let called = self.called.clone();
            async move {
                called.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_run_machine_stdio_server_delegates_to_api_transport() -> Result<()> {
        let called = Arc::new(AtomicBool::new(false));
        let fixture = MockApi::new(called.clone());

        run_machine_stdio_server(&fixture).await?;

        let actual = called.load(Ordering::SeqCst);
        let expected = true;
        assert_eq!(actual, expected);
        Ok(())
    }
}
