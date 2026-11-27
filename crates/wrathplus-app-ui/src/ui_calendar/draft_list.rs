//! A draft's invite-list edits. 12340 changes a draft's list in place and fires
//! `CALENDAR_UPDATE_INVITE_LIST` at the edit (status `0x5f2784`, moderator `0x5f28cf`, removal
//! `0x5b97b4`); here App holds the draft, so the event fires once App has applied the edit.

use wrathplus_ui::script::WrathCalendarRequest as R;

pub(super) fn redraws(request: &R) -> bool {
    matches!(
        request,
        R::SetStatus { event_id: 0, .. }
            | R::SetModerator { event_id: 0, .. }
            | R::RemoveInvite { event_id: 0, .. }
    )
}
