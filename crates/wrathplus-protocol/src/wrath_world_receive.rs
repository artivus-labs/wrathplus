//! A world receive's result once its socket's read timeout is put back (`wrath_world`).

use anyhow::{Context, Result};
use std::io;

/// After a receive with a deadline, restoring the socket's own read timeout.
pub(super) const DEADLINE: &str = "restoring Wrath world socket timeout after deadline receive";
/// After a receive with a frame timeout, restoring the socket's idle timeout.
pub(super) const IDLE: &str = "restoring Wrath world socket idle timeout after frame receive";

/// A socket whose read timeout can't be restored is unusable (`failed`). When the receive itself
/// failed, its error (a reset, EOF) stays the cause, the failed restore added to it: macOS refuses
/// the timeout (EINVAL) on a socket whose peer reset it, and reporting only that hid why the
/// connection dropped.
pub(super) fn after_receive<T>(
    result: Result<T>,
    restored: io::Result<()>,
    failed: &mut bool,
    restoring: &'static str,
) -> Result<T> {
    let Err(error) = restored else { return result };
    *failed = true;
    match result {
        Err(received) => Err(received.context(format!("{restoring} also failed: {error}"))),
        Ok(_) => Err(error).context(restoring),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() -> Result<u8> {
        Err(io::Error::from(io::ErrorKind::ConnectionReset).into())
    }

    fn einval() -> io::Result<()> {
        Err(io::Error::from_raw_os_error(22))
    }

    #[test]
    fn a_failed_receive_keeps_its_cause_when_the_timeout_cannot_be_restored() {
        let mut failed = false;
        let err = after_receive(reset(), einval(), &mut failed, IDLE).unwrap_err();
        assert!(failed);
        let cause = err.chain().find_map(|e| e.downcast_ref::<io::Error>());
        assert_eq!(
            cause.map(io::Error::kind),
            Some(io::ErrorKind::ConnectionReset)
        );
        assert!(
            format!("{err:#}").starts_with(&format!("{IDLE} also failed: ")),
            "{err:#}"
        );
    }

    #[test]
    fn a_receive_is_lost_with_its_socket_and_kept_when_the_timeout_is_restored() {
        let mut failed = false;
        assert!(after_receive(Ok(1), einval(), &mut failed, DEADLINE).is_err() && failed);
        let mut failed = false;
        assert_eq!(
            after_receive(Ok(1), Ok(()), &mut failed, DEADLINE).unwrap(),
            1
        );
        assert!(after_receive(reset(), Ok(()), &mut failed, DEADLINE).is_err() && !failed);
    }
}
