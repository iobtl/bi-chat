use tokio::sync::{
    broadcast::{self, error::TryRecvError},
    mpsc,
};

pub type ShutdownListener = broadcast::Receiver<()>;

pub struct Shutdown {
    // Should be true when shutdown signal has been received by broadcast
    shutdown: bool,

    // Listen for shutdown signal
    shutdown_listener: ShutdownListener,

    // Propagates to server whether process has successfully terminated
    _shutdown_complete_tx: mpsc::Sender<()>,
}

impl Shutdown {
    pub fn new(shutdown_listener: ShutdownListener, shutdown_complete: mpsc::Sender<()>) -> Self {
        Shutdown {
            shutdown: false,
            shutdown_listener,
            _shutdown_complete_tx: shutdown_complete,
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    pub async fn async_listen(&mut self) {
        if self.shutdown {
            return;
        }

        // Yields until shutdown signal received
        let _ = self.shutdown_listener.recv().await;

        // Shutdown signal received
        self.shutdown = true;
    }

    pub fn listen(&mut self) {
        if self.shutdown {
            return;
        }

        match self.shutdown_listener.try_recv() {
            // Either shutdown signal recevied or all senders have been dropped
            Ok(_) | Err(TryRecvError::Closed) => {
                self.shutdown = true;
            }

            // No shutdown signal received yet
            Err(TryRecvError::Empty) => {}

            // It should not be possible for this channel to lag
            // Since the only value that is ever sent will be the shutdown signal
            Err(TryRecvError::Lagged(_)) => {}
        }
    }
}
