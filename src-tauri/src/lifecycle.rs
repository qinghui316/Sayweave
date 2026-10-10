//! Atomic application admission, independent of service-owned task state.
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Lifecycle(Arc<Mutex<Admission>>);
#[derive(Default)]
struct Admission {
    fenced: bool,
    active: usize,
}
pub struct Permit(Arc<Mutex<Admission>>);
impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.active -= 1;
        }
    }
}
impl Lifecycle {
    pub fn enter(&self) -> Result<Permit, &'static str> {
        let mut state = self.0.lock().map_err(|_| "unavailable")?;
        if state.fenced {
            return Err("stopping");
        }
        state.active += 1;
        Ok(Permit(self.0.clone()))
    }
    /// A start must hold a permit until its service registers the busy task.
    pub fn fence(&self, busy: impl FnOnce() -> bool) -> Result<(), &'static str> {
        let mut state = self.0.lock().map_err(|_| "unavailable")?;
        if state.fenced || busy() {
            return Err("busy");
        }
        state.fenced = true;
        Ok(())
    }
    pub fn release(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.fenced = false;
        }
    }
    pub async fn drain(&self) -> Result<(), &'static str> {
        loop {
            if self.0.lock().map_err(|_| "unavailable")?.active == 0 {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
pub fn mutation(window: &tauri::WebviewWindow) -> Result<Permit, crate::data::model::DataError> {
    use tauri::Manager;
    window
        .app_handle()
        .state::<Lifecycle>()
        .enter()
        .map_err(crate::data::model::DataError::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fence_waits_admitted_writes_and_rejects_new_starts() {
        let gate = Lifecycle::default();
        let write = gate.enter().unwrap();
        gate.fence(|| false).unwrap();
        assert!(gate.enter().is_err());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), gate.drain())
                .await
                .is_err()
        );
        drop(write);
        gate.drain().await.unwrap();
        gate.release();
        assert!(gate.enter().is_ok());
        assert!(gate.fence(|| true).is_err());
    }
}
