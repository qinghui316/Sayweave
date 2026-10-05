use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Connecting,
    Recording,
    Processing,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Windows {
    pub main: bool,
    pub overlay: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: u64,
    pub phase: Phase,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub protocol: u8,
    pub revision: u64,
    pub windows: Windows,
    pub session: Session,
}
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeError {
    pub code: &'static str,
}
impl RuntimeError {
    pub fn new(code: &'static str) -> Self {
        Self { code }
    }
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            protocol: 1,
            revision: 1,
            windows: Windows {
                main: true,
                overlay: false,
            },
            session: Session {
                id: 0,
                phase: Phase::Idle,
            },
        }
    }
}
impl Snapshot {
    pub fn demo(&mut self, action: &str, id: u64) -> Result<(), RuntimeError> {
        use Phase::*;
        let active = matches!(self.session.phase, Connecting | Recording | Processing);
        let phase = match action {
            "start" if !active => {
                self.session.id += 1;
                Connecting
            }
            "start" => return Err(RuntimeError::new("invalidAction")),
            _ if id != self.session.id => return Err(RuntimeError::new("staleSession")),
            "finish" if self.session.phase == Recording => Processing,
            "cancel" if active => Cancelled,
            "fail" if active => Failed,
            _ => return Err(RuntimeError::new("invalidAction")),
        };
        self.session.phase = phase;
        self.revision += 1;
        Ok(())
    }
    pub fn settle(&mut self, id: u64, expected: Phase, phase: Phase) -> bool {
        if self.session.id != id || self.session.phase != expected {
            return false;
        }
        self.session.phase = phase;
        self.revision += 1;
        true
    }
    pub fn window(&mut self, surface: &str, visible: bool) -> Result<(), RuntimeError> {
        match surface {
            "main" => self.windows.main = visible,
            "overlay" => self.windows.overlay = visible,
            _ => return Err(RuntimeError::new("invalidAction")),
        }
        self.revision += 1;
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.session.id += 1;
        self.session.phase = Phase::Idle;
        self.windows.main = false;
        self.windows.overlay = false;
        self.revision += 1;
    }
}
pub fn authorized_window(caller: &str, surface: &str, action: &str) -> bool {
    caller == "main" || (caller == "voice-overlay" && surface == "overlay" && action == "hide")
}
pub fn overlay_size(_phase: Phase) -> (f64, f64) {
    // The compact shell animates inside this canvas, keeping its screen anchor stable.
    (160.0, 88.0)
}
pub fn reserve_size(current: (f64, f64), target: (f64, f64)) -> (f64, f64) {
    (current.0.max(target.0), current.1.max(target.1))
}
pub fn resize_is_current(current: &Snapshot, requested: &Snapshot) -> bool {
    current.revision == requested.revision && current.session == requested.session
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resize_reserves_each_axis_and_rejects_stale_session_or_revision() {
        assert_eq!(reserve_size((240.0, 76.0), (108.0, 64.0)), (240.0, 76.0));
        assert_eq!(reserve_size((108.0, 64.0), (68.0, 68.0)), (108.0, 68.0));
        assert_eq!(reserve_size((68.0, 68.0), (240.0, 76.0)), (240.0, 76.0));
        let mut current = Snapshot::default();
        let requested = current.clone();
        assert!(resize_is_current(&current, &requested));
        current.window("main", false).unwrap();
        assert!(!resize_is_current(&current, &requested));
        current.revision = requested.revision;
        current.session.id += 1;
        assert!(!resize_is_current(&current, &requested));
    }
    #[test]
    fn cancelled_and_new_sessions_reject_late_completion() {
        let mut s = Snapshot::default();
        s.demo("start", 0).unwrap();
        let old = s.session.id;
        s.settle(old, Phase::Connecting, Phase::Recording);
        s.demo("finish", old).unwrap();
        s.demo("cancel", old).unwrap();
        assert!(!s.settle(old, Phase::Processing, Phase::Completed));
        s.demo("start", old).unwrap();
        assert!(!s.settle(old, Phase::Connecting, Phase::Recording));
        assert_eq!(s.demo("finish", old).unwrap_err().code, "staleSession");
    }
    #[test]
    fn hidden_main_does_not_own_demo_lifetime() {
        let mut s = Snapshot::default();
        s.demo("start", 0).unwrap();
        let id = s.session.id;
        s.window("main", false).unwrap();
        assert!(s.settle(id, Phase::Connecting, Phase::Recording));
        assert!(!s.windows.main);
    }
    #[test]
    fn invalid_actions_preserve_revision_and_shutdown_invalidates_tasks() {
        let mut s = Snapshot::default();
        assert!(s.demo("finish", 0).is_err());
        assert_eq!(s.revision, 1);
        s.demo("start", 0).unwrap();
        let id = s.session.id;
        s.shutdown();
        assert!(!s.settle(id, Phase::Connecting, Phase::Recording));
    }
    #[test]
    fn overlay_cannot_operate_main() {
        assert!(authorized_window("voice-overlay", "overlay", "hide"));
        assert!(!authorized_window("voice-overlay", "main", "show"));
        assert!(!authorized_window("voice-overlay", "overlay", "show"));
    }
    #[test]
    fn serialization_matches_shared_fixture() {
        let fixture: Snapshot =
            serde_json::from_str(include_str!("../../tests/contracts/snapshot.json")).unwrap();
        assert_eq!(fixture.protocol, 1);
        assert_eq!(fixture.session.phase, Phase::Recording);
        assert_eq!(
            serde_json::to_value(fixture).unwrap(),
            serde_json::from_str::<serde_json::Value>(include_str!(
                "../../tests/contracts/snapshot.json"
            ))
            .unwrap()
        );
    }
}
