//! Bounded owned-fixture timing controls; absent from ordinary DLLs.
use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
static FAULT: AtomicU32 = AtomicU32::new(0);
static NATIVE_SCOPE_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
thread_local! {static DEFERRED:RefCell<Vec<(ITfContext,u32,ITfEditSession,TF_CONTEXT_EDIT_CONTEXT_FLAGS)>>=const{RefCell::new(Vec::new())};}
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveBlockFixtureFault(value: u32) {
    if value == 11 {
        NATIVE_SCOPE_ONLY.store(true, Ordering::Release);
        return;
    }
    FAULT.store(value, Ordering::Release);
}
pub fn native_scope_only() -> bool {
    NATIVE_SCOPE_ONLY.load(Ordering::Acquire)
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
pub fn reject_owned_follow() -> bool {
    FAULT
        .compare_exchange(10, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}
pub fn acquisition_without_object(shared: &Shared) -> windows::core::Result<()> {
    let fault = FAULT.load(Ordering::Acquire);
    if matches!(fault, 7..=9)
        && FAULT
            .compare_exchange(fault, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        if fault == 9 {
            // The same candidate resources survive a synchronous revocation.
            shared.borrow_mut().generation = next_generation();
        }
        return Err(failure(if fault == 8 { E_POINTER } else { E_FAIL }));
    }
    Ok(())
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
