use tokio::sync::mpsc::Sender;

pub struct SingleBufferSender<T> {
    sender: Sender<T>,
    unsent: Option<T>,
}

pub type TrySendError = ();

impl<T> SingleBufferSender<T> {
    pub fn new(sender: Sender<T>) -> Self {
        SingleBufferSender {
            unsent: None,
            sender,
        }
    }

    pub fn send_or_store(&mut self, value: T) -> Result<bool, TrySendError> {
        debug_assert!(self.unsent.is_none());
        match self.sender.try_send(value) {
            Ok(_) => Ok(true),
            Err(e) => match e {
                tokio::sync::mpsc::error::TrySendError::Full(value) => {
                    self.unsent = Some(value);
                    Ok(false)
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => Err(()),
            },
        }
    }

    pub fn try_flush(&mut self) -> Result<bool, TrySendError> {
        match self.unsent.take() {
            None => Ok(true),
            Some(value) => self.send_or_store(value),
        }
    }
}
