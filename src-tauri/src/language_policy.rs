//! Runtime publication of independent learning policies; no model, data or native access.
use crate::contracts::language::{LanguageSettings, LearningEligibility, Settings};
use std::future::Future;
use tokio::sync::watch;

pub(crate) struct LearningControl {
    changed: watch::Sender<LanguageSettings>,
}
impl Default for LearningControl {
    fn default() -> Self {
        Self::new(LanguageSettings {
            automatic_terms: Settings {
                enabled: false,
                revision: 0,
            },
            personalization: Settings {
                enabled: false,
                revision: 0,
            },
        })
    }
}
impl LearningControl {
    pub fn new(settings: LanguageSettings) -> Self {
        Self {
            changed: watch::channel(settings).0,
        }
    }
    pub fn snapshot(&self) -> LanguageSettings {
        self.changed.borrow().clone()
    }
    pub fn publish(&self, value: &LanguageSettings) {
        self.changed.send_if_modified(|current| {
            let mut changed = false;
            for (old, new) in [
                (&mut current.automatic_terms, &value.automatic_terms),
                (&mut current.personalization, &value.personalization),
            ] {
                if new.revision > old.revision {
                    *old = new.clone();
                    changed = true;
                }
            }
            changed
        });
    }
    pub fn live(&self, interest: &LearningEligibility) -> LearningEligibility {
        interest.live(&self.snapshot())
    }
    pub fn any(&self) -> bool {
        LearningEligibility::capture(&self.snapshot()).any()
    }
    pub async fn while_valid<T>(
        &self,
        interest: &LearningEligibility,
        future: impl Future<Output = T>,
    ) -> Result<T, ()> {
        let mut changes = self.changed.subscribe();
        tokio::pin!(future);
        loop {
            if !interest.live(&changes.borrow_and_update()).any() {
                return Err(());
            }
            tokio::select! { biased;
                result = changes.changed() => { if result.is_err() { return Err(()); } }
                value = &mut future => return Ok(value),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn first() -> LanguageSettings {
        LanguageSettings {
            automatic_terms: Settings {
                enabled: true,
                revision: 1,
            },
            personalization: Settings {
                enabled: true,
                revision: 1,
            },
        }
    }
    #[tokio::test]
    async fn shared_request_survives_one_disabled_feature_and_cancels_only_when_none_remain() {
        let control = Arc::new(LearningControl::new(first()));
        let interest = LearningEligibility::capture(&first());
        let (ready, started) = tokio::sync::oneshot::channel();
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        let worker = control.clone();
        let task = tokio::spawn(async move {
            worker
                .while_valid(&interest, async move {
                    ready.send(()).unwrap();
                    receive.await
                })
                .await
        });
        started.await.unwrap();
        let mut settings = first();
        settings.personalization = Settings {
            enabled: false,
            revision: 2,
        };
        control.publish(&settings);
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        settings.automatic_terms = Settings {
            enabled: false,
            revision: 2,
        };
        control.publish(&settings);
        assert!(task.await.unwrap().is_err());
        assert!(send.send(()).is_err());
    }
    #[tokio::test]
    async fn style_request_is_unchanged_by_dictionary_off_on_and_old_tokens_are_not_reauthorized() {
        let control = Arc::new(LearningControl::new(first()));
        let interest = LearningEligibility {
            automatic_terms: None,
            personalization: Some(1),
        };
        let (ready, started) = tokio::sync::oneshot::channel();
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        let worker = control.clone();
        let task = tokio::spawn(async move {
            worker
                .while_valid(&interest, async move {
                    ready.send(()).unwrap();
                    receive.await
                })
                .await
        });
        started.await.unwrap();
        let mut value = first();
        for (enabled, revision) in [(false, 2), (true, 3)] {
            value.automatic_terms = Settings { enabled, revision };
            control.publish(&value);
        }
        control.publish(&first());
        assert_eq!(control.snapshot().automatic_terms.revision, 3);
        assert!(!task.is_finished());
        send.send(()).unwrap();
        assert!(task.await.unwrap().unwrap().is_ok());
        let old = LearningEligibility {
            automatic_terms: Some(1),
            personalization: None,
        };
        assert!(!control.live(&old).any());
        let polled = std::sync::atomic::AtomicBool::new(false);
        assert!(
            control
                .while_valid(&old, async {
                    polled.store(true, std::sync::atomic::Ordering::Release)
                })
                .await
                .is_err()
        );
        assert!(!polled.load(std::sync::atomic::Ordering::Acquire));
    }
}
