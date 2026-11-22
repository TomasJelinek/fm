//! The notice in an HTML page's place when the system refused its browser the sandbox, with
//! Install… where an administrator can install the permission from here, or in a
//! document's place when it failed with diagnostic details, with Copy details. The backend
//! decides what it says, whether Install… is offered and runs the installation; this only
//! presents the notice and forwards the requests.
use super::*;

pub(super) fn present(open: &OpenWindow, notice: Option<&ViewerSandboxNotice>) {
    let window = &open.window;
    // Install… may hold the focus when the notice goes with the document that opens
    // instead; the page then gets it, unless Find is in use.
    let refocus = notice.is_none()
        && window.get_sandbox_notice_visible()
        && !window.get_content_focused()
        && !window.get_search_visible();
    window.set_sandbox_notice(notice.map_or_else(Default::default, |notice| {
        midnight_core::visible_name(&notice.message).as_ref().into()
    }));
    // Copy details copies the raw message and details, as other copy actions do.
    window.set_sandbox_details(
        notice
            .and_then(|notice| {
                let details = notice.details.as_ref()?;
                Some(format!("{}\n\n{details}", notice.message))
            })
            .unwrap_or_default()
            .into(),
    );
    window.set_sandbox_installable(notice.is_some_and(|notice| notice.installable));
    window.set_sandbox_installing(notice.is_some_and(|notice| notice.installing));
    window.set_sandbox_notice_visible(notice.is_some());
    if refocus {
        window.invoke_focus_content();
    }
}

/// Copy details: the message, a blank line and the details, through the viewer's copy,
/// which hands them to the engine's clipboard command.
pub(super) fn bind_copy(open: &Rc<OpenWindow>, copy: Rc<dyn Fn(String)>) {
    let weak = Rc::downgrade(open);
    open.window.on_copy_sandbox_details(move || {
        if let Some(open) = weak.upgrade() {
            let text = open.window.get_sandbox_details();
            if !text.is_empty() {
                copy(text.into());
            }
        }
    });
}

pub(super) fn bind(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_install_sandbox_permission(move || {
        if let Some(open) = weak.upgrade()
            && open.window.get_sandbox_installable()
            && !open.window.get_sandbox_installing()
        {
            open.enqueue(ViewerInput::InstallSandboxPermission);
        }
    });
}
