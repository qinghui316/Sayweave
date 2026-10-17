//! Retained native document identity for a continuously tracked block.
//! An accessibility provider supplements this identity; it is not a text decoder.
use super::*;

#[derive(Clone)]
pub(super) struct BlockScope {
    manager: ITfThreadMgr,
    document: ITfDocumentMgr,
    context: ITfContext,
    view: ITfContextView,
    window: HWND,
    root: HWND,
    field: FieldProof,
}

#[derive(Clone)]
enum FieldProof {
    Accessible(initial_selection::FocusIdentity),
    // Captured only from the writer's already-owned composition, never from
    // an arbitrary caret or an empty accessibility identity. Document/context
    // lifetime events irrevocably invalidate the Track which retains this.
    NativeOwned,
}

impl BlockScope {
    pub(super) unsafe fn capture(
        manager: &ITfThreadMgr,
        context: &ITfContext,
    ) -> windows::core::Result<Self> {
        unsafe {
            let document = manager.GetFocus()?;
            if document.GetTop()? != *context || context.GetDocumentMgr()? != document {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let window = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
            let mut process = 0;
            if window.0.is_null()
                || GetWindowThreadProcessId(window, Some(&mut process))
                    != windows::Win32::System::Threading::GetCurrentThreadId()
                || process != std::process::id()
            {
                return Err(failure(E_ACCESSDENIED));
            }
            let root = GetAncestor(window, GA_ROOT);
            let view = context.GetActiveView()?;
            let view_window = view.GetWnd()?;
            if root.0.is_null()
                || root != GetForegroundWindow()
                || (!view_window.0.is_null() && GetAncestor(view_window, GA_ROOT) != root)
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            #[cfg(feature = "acceptance")]
            let native_only = block_faults::native_scope_only();
            #[cfg(not(feature = "acceptance"))]
            let native_only = false;
            let field = if native_only {
                FieldProof::NativeOwned
            } else {
                match initial_selection::FocusIdentity::capture() {
                    Ok(identity) => FieldProof::Accessible(identity),
                    Err(error) if error.code() == E_NOINTERFACE => FieldProof::NativeOwned,
                    Err(error) => return Err(error),
                }
            };
            let scope = Self {
                manager: manager.clone(),
                document,
                context: context.clone(),
                view,
                window,
                root,
                field,
            };
            if !scope.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Ok(scope)
        }
    }

    pub(super) fn matches(&self) -> windows::core::Result<bool> {
        unsafe {
            if !IsWindow(Some(self.window)).as_bool()
                || windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() != self.window
                || GetAncestor(self.window, GA_ROOT) != self.root
                || GetForegroundWindow() != self.root
                || self.manager.GetFocus()?.cast::<IUnknown>()?
                    != self.document.cast::<IUnknown>()?
                || self.document.GetTop()?.cast::<IUnknown>()? != self.context.cast::<IUnknown>()?
                || self.context.GetActiveView()?.cast::<IUnknown>()?
                    != self.view.cast::<IUnknown>()?
            {
                return Ok(false);
            }
            let window = self.view.GetWnd()?;
            if !window.0.is_null() && GetAncestor(window, GA_ROOT) != self.root {
                return Ok(false);
            }
            match &self.field {
                FieldProof::Accessible(field) => field.matches(),
                FieldProof::NativeOwned => Ok(true),
            }
        }
    }

    pub(super) fn native_identity_text(&self) -> bool {
        matches!(&self.field, FieldProof::Accessible(field) if field.native_identity_text())
    }
    pub(super) fn has_native_origin(&self) -> bool {
        matches!(self.field, FieldProof::NativeOwned)
    }
}
