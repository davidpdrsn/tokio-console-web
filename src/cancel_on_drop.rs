pub struct CancelOnDrop {
    inner: tokio_util::sync::CancellationToken,
}

impl CancelOnDrop {
    pub fn new() -> Self {
        Self {
            inner: tokio_util::sync::CancellationToken::new(),
        }
    }

    pub fn child(&self) -> CancelOnDropChildToken {
        CancelOnDropChildToken {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.inner.cancel();
    }
}

pub struct CancelOnDropChildToken {
    inner: tokio_util::sync::CancellationToken,
}

impl CancelOnDropChildToken {
    pub async fn cancelled(self) {
        self.inner.cancelled().await
    }
}
