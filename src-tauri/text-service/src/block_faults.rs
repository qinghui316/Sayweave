//! Bounded owned-fixture timing controls; absent from ordinary DLLs.
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
static FAULT: AtomicU32 = AtomicU32::new(0);
thread_local! {static DEFERRED:RefCell<Vec<(ITfContext,u32,ITfEditSession,TF_CONTEXT_EDIT_CONTEXT_FLAGS)>>=const{RefCell::new(Vec::new())};}
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveBlockFixtureFault(value: u32) {
    FAULT.store(value, Ordering::Release);
}
pub fn coarse_notification(text_changed: bool) -> bool {
    text_changed
        && FAULT
            .compare_exchange(5, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
}
pub fn stale_tracking_range() -> bool {
    FAULT
        .compare_exchange(6, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}
pub fn request(
    context: &ITfContext,
    client: u32,
    edit: &ITfEditSession,
    flags: TF_CONTEXT_EDIT_CONTEXT_FLAGS,
) -> Option<windows::core::Result<HRESULT>> {
    let fault = FAULT.load(Ordering::Acquire);
    if !matches!(fault, 1 | 3 | 4)
        || FAULT
            .compare_exchange(fault, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return None;
    }
    Some(match fault {
        1 => {
            DEFERRED.with(|q| {
                q.borrow_mut()
                    .push((context.clone(), client, edit.clone(), flags))
            });
            Ok(S_OK)
        }
        3 => Err(failure(E_FAIL)),
        _ => Ok(E_FAIL),
    })
}
pub fn started(shared: &Shared, r: &Request) {
    if FAULT
        .compare_exchange(2, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let revoke = Request {
        action: Action::Revoke,
        operation: r.operation + 1,
        text: String::new(),
        block: None,
        prefix: None,
        revoke: Some(crate::wire::Revoke {
            operation: r.operation,
            revision: r.revision,
        }),
        ..r.clone()
    };
    let connection = shared.borrow().connection;
    revoke_operation(shared, &revoke, connection);
}
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveBlockFixtureFlush() -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let requests = DEFERRED.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for (context, client, edit, flags) in requests {
            unsafe {
                let _ = context.RequestEditSession(client, &edit, flags);
            }
        }
    }))
    .is_ok()
}
