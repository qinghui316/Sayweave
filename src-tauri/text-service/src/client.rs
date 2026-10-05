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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeechActivation {
    Inactive,
    Own,
    Other,
}
fn classify_speech(profiles: &[TF_INPUTPROCESSORPROFILE], language: u16) -> SpeechActivation {
    let mut own = false;
    for p in profiles
        .iter()
        .filter(|p| p.catid == GUID_TFCAT_TIP_SPEECH && p.dwFlags & TF_IPP_FLAG_ACTIVE != 0)
    {
        if p.clsid != registration::CLASS
            || p.guidProfile != registration::PROFILE
            || p.langid != language
            || p.dwProfileType != TF_PROFILETYPE_INPUTPROCESSOR
        {
            return SpeechActivation::Other;
        }
        own = true;
    }
    if own {
        SpeechActivation::Own
    } else {
        SpeechActivation::Inactive
    }
}
pub fn speech_activation() -> Result<(SpeechActivation, u16)> {
    let language = unsafe { registration::profiles()?.GetCurrentLanguage()? };
    Ok((classify_speech(&input_profiles()?, language), language))
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
        // Enabling our user profile can leave it active before any dictation starts.
        // HostLease excludes another Sayweave host; preserve a pre-existing activation.
        let (activation, language) = speech_activation()?;
        if activation == SpeechActivation::Other {
            return Err(fail("another speech profile is already active"));
        }
        let reused = activation == SpeechActivation::Own;
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
        if !reused {
            // Only the activation created by this lease is undone by close().
            lease.language = Some(language);
            lease.language = Some(registration::activate(true)?);
        }
        if !own_profile_active(language)? {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_activation_requires_complete_own_identity() {
        let own = TF_INPUTPROCESSORPROFILE {
            dwProfileType: TF_PROFILETYPE_INPUTPROCESSOR,
            langid: 2052,
            clsid: registration::CLASS,
            guidProfile: registration::PROFILE,
            catid: GUID_TFCAT_TIP_SPEECH,
            dwFlags: TF_IPP_FLAG_ACTIVE,
            ..Default::default()
        };
        assert_eq!(classify_speech(&[], 2052), SpeechActivation::Inactive);
        assert_eq!(classify_speech(&[own], 2052), SpeechActivation::Own);
        assert_eq!(classify_speech(&[own], 1033), SpeechActivation::Other);
        for changed in [
            TF_INPUTPROCESSORPROFILE {
                clsid: GUID::zeroed(),
                ..own
            },
            TF_INPUTPROCESSORPROFILE {
                guidProfile: GUID::zeroed(),
                ..own
            },
            TF_INPUTPROCESSORPROFILE {
                dwProfileType: 0,
                ..own
            },
        ] {
            assert_eq!(
                classify_speech(&[own, changed], 2052),
                SpeechActivation::Other
            );
        }
    }
}
