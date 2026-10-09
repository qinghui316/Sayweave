//! A single monotonic stop signal. An unread user cancel survives shutdown.
pub(crate) enum OptionalRead<T> {
    Ready(T),
    Unavailable,
    Stopped,
}
pub(crate) async fn optional_read<T>(
    future: impl std::future::Future<Output = T>,
    mut cancel: tokio::sync::watch::Receiver<u8>,
    budget: std::time::Duration,
) -> OptionalRead<T> {
    if *cancel.borrow() > 0 {
        return OptionalRead::Stopped;
    }
    tokio::select! {biased;
        _=cancel.changed()=>OptionalRead::Stopped,
        value=tokio::time::timeout(budget,future)=>match value {Ok(v)=>OptionalRead::Ready(v),Err(_)=>OptionalRead::Unavailable},
    }
}
pub(crate) fn request(
    sender: &tokio::sync::watch::Sender<u8>,
    reason: crate::contracts::speech::StopReason,
) -> Result<(), ()> {
    if sender.receiver_count() == 0 {
        return Err(());
    }
    sender.send_if_modified(|current| {
        let next = reason.merge(*current);
        if next == *current {
            false
        } else {
            *current = next;
            true
        }
    });
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::speech::StopReason::*;
    #[tokio::test]
    async fn cancelled_optional_preparation_drops_slow_read_before_next_admission() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct ReadLease(Arc<AtomicBool>);
        impl Drop for ReadLease {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let ended = Arc::new(AtomicBool::new(false));
        let lease = ReadLease(ended.clone());
        let (tx, rx) = tokio::sync::watch::channel(0);
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(optional_read(
            async move {
                let _lease = lease;
                let _ = started.send(());
                std::future::pending::<()>().await;
            },
            rx,
            std::time::Duration::from_secs(2),
        ));
        ready.await.unwrap();
        request(&tx, Cancel).unwrap();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), task)
                .await
                .unwrap()
                .unwrap(),
            OptionalRead::Stopped
        ));
        assert!(ended.load(Ordering::Acquire));
        let (_tx, rx) = tokio::sync::watch::channel(0);
        assert!(matches!(
            optional_read(
                std::future::pending::<()>(),
                rx,
                std::time::Duration::from_millis(10)
            )
            .await,
            OptionalRead::Unavailable
        ));
    }
    #[test]
    fn unread_cancel_is_not_downgraded_by_finish_or_shutdown() {
        let (signal, receiver) = tokio::sync::watch::channel(0);
        request(&signal, Cancel).unwrap();
        request(&signal, Interrupted).unwrap();
        request(&signal, Finish).unwrap();
        assert_eq!(*receiver.borrow(), 2);
        let (signal, receiver) = tokio::sync::watch::channel(0);
        request(&signal, Interrupted).unwrap();
        request(&signal, Cancel).unwrap();
        assert_eq!(*receiver.borrow(), 2);
    }
}
