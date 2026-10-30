//! AzerothCore transport clock precision, shared with the native timetable builder.
//!
//! The server uses `TransportMgr::GeneratePath`'s final DepartureTime as pathTime, not the
//! stock client's independently computed/sniffed period. Mixing those clocks drifts on the
//! serialized `MotionTransport::PathProgress` anchor.

/// `Movement/Spline/Spline.h`: AC measures each Catmull-Rom segment over three chords.
pub(crate) const AC_CHORDS_PER_SEGMENT: u32 = 3;
