//! A draft's invite list. 12340 builds and edits a draft's list in place and fires
//! `CALENDAR_UPDATE_INVITE_LIST` at once: with `true` for a new player draft (`0x5bbf05`, the
//! guild drafts fire nothing), bare at an edit (status `0x5f2784`, moderator `0x5f28cf`, removal
//! `0x5b97b4`). Here App holds the draft, so the event fires once App has applied the request.

use wrathplus_ui::script::{ScriptValue as V, WrathCalendarRequest as R};

pub(super) fn redraws(request: &R) -> Option<Vec<V>> {
    match request {
        R::NewDraft { flags, .. } => (flags & 0x440 == 0).then(|| vec![V::Bool(true)]),
        R::SetStatus { event_id: 0, .. }
        | R::SetModerator { event_id: 0, .. }
        | R::RemoveInvite { event_id: 0, .. } => Some(vec![]),
        _ => None,
    }
}
