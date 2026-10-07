//! A single monotonic stop signal. An unread user cancel survives shutdown.
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
