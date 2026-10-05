//! Main-process activation lease. Call on one COM apartment; never change keyboard defaults.
use crate::registration;
use windows::{
    Win32::{Foundation::E_FAIL, System::Com::*, UI::TextServices::*},
    core::{GUID, Interface, Result},
};

fn fail(message: &str) -> windows::core::Error {
    windows::core::Error::new(E_FAIL, message)
}
pub fn input_profiles() -> Result<Vec<TF_INPUTPROCESSORPROFILE>> {
    unsafe {
        let manager: ITfInputProcessorProfileMgr = registration::profiles()?.cast()?;
        let enumerator = manager.EnumProfiles(0)?;
        let mut result = Vec::new();
        loop {
            let mut batch = [TF_INPUTPROCESSORPROFILE::default(); 16];
            let mut fetched = 0;
            enumerator.Next(&mut batch, &mut fetched)?;
            if fetched as usize > batch.len() || result.len() + fetched as usize > 512 {
                return Err(fail("profile enumeration exceeded bounds"));
            }
            result.extend_from_slice(&batch[..fetched as usize]);
            if fetched == 0 {
                return Ok(result);
            }
        }
    }
}
pub fn own_profile_active(language: u16) -> Result<bool> {
    unsafe {
        let mut active_language = 0;
        let mut active_profile = GUID::zeroed();
        registration::profiles()?.GetActiveLanguageProfile(
            &registration::CLASS,
            &mut active_language,
            &mut active_profile,
        )?;
        Ok(active_language == language && active_profile == registration::PROFILE)
    }
}
type Keyboard = (u32, u16, GUID, GUID, isize);
fn keyboard() -> Result<Keyboard> {
    unsafe {
        let manager: ITfInputProcessorProfileMgr = registration::profiles()?.cast()?;
        let mut profile = TF_INPUTPROCESSORPROFILE::default();
        manager.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut profile)?;
        Ok((
            profile.dwProfileType,
            profile.langid,
            profile.clsid,
            profile.guidProfile,
            profile.hkl.0 as isize,
        ))
    }
}
pub struct ProfileLease {
    manager: Option<ITfThreadMgr>,
    language: Option<u16>,
}
impl ProfileLease {
    /// Caller must already hold HostLease, serializing activation with all other hosts.
    pub fn open() -> Result<Self> {
        if input_profiles()?
            .iter()
            .any(|p| p.catid == GUID_TFCAT_TIP_SPEECH && p.dwFlags & TF_IPP_FLAG_ACTIVE != 0)
        {
            return Err(fail("speech profile is already active"));
        }
        let before = keyboard()?;
        let mut lease = Self {
            manager: None,
            language: None,
        };
        unsafe {
            let manager: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
            manager.Activate()?;
            lease.manager = Some(manager);
        }
        // Record the language before activation, so even a partially failed activation cleans up.
        lease.language = Some(unsafe { registration::profiles()?.GetCurrentLanguage()? });
        // activate resolves the current language itself; retain the actual successful identity.
        lease.language = Some(registration::activate(true)?);
        if !own_profile_active(lease.language.unwrap())? {
            return Err(fail("speech activation not confirmed"));
        }
        if keyboard()? != before {
            return Err(fail("speech activation changed keyboard profile"));
        }
        Ok(lease)
    }
    pub fn close(&mut self) -> Result<()> {
        if let Some(language) = self.language {
            // Never deactivate a replacement profile selected by the user.
            if own_profile_active(language)? {
                registration::deactivate(language, true)?;
                if own_profile_active(language)? {
                    return Err(fail("speech profile remains active"));
                }
            }
            self.language = None;
        }
        if let Some(manager) = &self.manager {
            unsafe {
                manager.Deactivate()?;
            }
            self.manager = None;
        }
        Ok(())
    }
}
impl Drop for ProfileLease {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
