use std::sync::{Arc, Mutex};

use bps_platform::{GlobalHotkeys, Hotkey, HotkeyId, PlatformError};

#[derive(Default)]
pub(crate) struct HotkeyState {
    /// 按注册顺序保存。
    registered: Vec<(HotkeyId, Hotkey)>,
}

impl HotkeyState {
    pub fn id_of(&self, hotkey: Hotkey) -> Option<HotkeyId> {
        self.registered
            .iter()
            .find(|(_, h)| *h == hotkey)
            .map(|(id, _)| *id)
    }

    pub fn hotkeys(&self) -> Vec<Hotkey> {
        self.registered.iter().map(|(_, h)| *h).collect()
    }
}

pub(crate) struct FakeHotkeys(pub Arc<Mutex<HotkeyState>>);

impl GlobalHotkeys for FakeHotkeys {
    fn register(&mut self, id: HotkeyId, hotkey: Hotkey) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        if state.registered.iter().any(|(i, _)| *i == id) {
            return Err(PlatformError::new(format!("热键标识 {id:?} 已被使用")));
        }
        if state.id_of(hotkey).is_some() {
            return Err(PlatformError::new(format!("热键 {hotkey:?} 已被注册")));
        }
        state.registered.push((id, hotkey));
        Ok(())
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        let mut state = self.0.lock().unwrap();
        let before = state.registered.len();
        state.registered.retain(|(i, _)| *i != id);
        if state.registered.len() == before {
            return Err(PlatformError::new(format!("热键标识 {id:?} 未注册")));
        }
        Ok(())
    }
}
