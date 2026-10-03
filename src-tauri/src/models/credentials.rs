use super::types::ModelError;
use std::sync::{Arc, Mutex};
pub trait Credentials: Send {
    fn get(&mut self, reference: &str) -> Result<String, ModelError>;
    fn put(&mut self, reference: &str, value: &str) -> Result<(), ModelError>;
    fn delete(&mut self, reference: &str) -> Result<(), ModelError>;
}
pub struct WindowsCredentials {
    namespace: String,
}
impl WindowsCredentials {
    pub fn new(namespace: String) -> Self {
        Self { namespace }
    }
}
impl Credentials for WindowsCredentials {
    fn get(&mut self, reference: &str) -> Result<String, ModelError> {
        #[cfg(target_os = "windows")]
        {
            keyring::Entry::new(&self.namespace, reference)
                .map_err(|_| ModelError::new("credentialUnavailable"))?
                .get_password()
                .map_err(|e| {
                    if matches!(e, keyring::Error::NoEntry) {
                        ModelError::new("credentialMissing")
                    } else {
                        ModelError::new("credentialUnavailable")
                    }
                })
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (&self.namespace, reference);
            Err(ModelError::new("credentialUnavailable"))
        }
    }
    fn put(&mut self, reference: &str, value: &str) -> Result<(), ModelError> {
        #[cfg(target_os = "windows")]
        {
            keyring::Entry::new(&self.namespace, reference)
                .map_err(|_| ModelError::new("credentialUnavailable"))?
                .set_password(value)
                .map_err(|_| ModelError::new("credentialUnavailable"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (reference, value);
            Err(ModelError::new("credentialUnavailable"))
        }
    }
    fn delete(&mut self, reference: &str) -> Result<(), ModelError> {
        #[cfg(target_os = "windows")]
        {
            match keyring::Entry::new(&self.namespace, reference)
                .map_err(|_| ModelError::new("credentialUnavailable"))?
                .delete_credential()
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(ModelError::new("credentialUnavailable")),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = reference;
            Err(ModelError::new("credentialUnavailable"))
        }
    }
}
#[derive(Clone)]
pub struct Vault(Arc<Mutex<Box<dyn Credentials>>>);
impl Vault {
    pub fn new(store: impl Credentials + 'static) -> Self {
        Self(Arc::new(Mutex::new(Box::new(store))))
    }
    async fn call<T: Send + 'static>(
        &self,
        op: impl FnOnce(&mut dyn Credentials) -> Result<T, ModelError> + Send + 'static,
    ) -> Result<T, ModelError> {
        let vault = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            op(vault
                .lock()
                .map_err(|_| ModelError::new("credentialUnavailable"))?
                .as_mut())
        })
        .await
        .map_err(|_| ModelError::new("credentialUnavailable"))?
    }
    pub async fn get(&self, r: String) -> Result<String, ModelError> {
        self.call(move |v| v.get(&r)).await
    }
    pub async fn put(&self, r: String, key: String) -> Result<(), ModelError> {
        self.call(move |v| v.put(&r, &key)).await
    }
    pub async fn delete(&self, r: String) -> Result<(), ModelError> {
        self.call(move |v| v.delete(&r)).await
    }
}
