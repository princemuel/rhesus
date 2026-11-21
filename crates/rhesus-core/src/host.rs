use alloc::string::String;
use alloc::vec::Vec;
use core::io::Write;

use crate::errors::HostError;

/// A minimal host only implements output.
pub trait Host {
    /// Writes `text` followed by a newline.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Io`] if the underlying writer fails.
    fn write_line(&mut self, text: &str) -> Result<(), HostError>;

    /// Reads the next line of input, without the trailing newline.
    /// `Ok(None)` means end of input.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Unsupported`] if the host has no input source
    /// (the default), or [`HostError::Io`] if reading fails.
    fn read_line(&mut self) -> Result<Option<String>, HostError> { Err(HostError::Unsupported) }

    /// Milliseconds from a host-defined epoch, for `clock()`.
    ///
    /// # Errors
    ///
    /// Returns [`HostError::Unsupported`] if the host has no clock (the
    /// default), or [`HostError::Io`] if reading it fails.
    fn now_millis(&self) -> Result<i64, HostError> { Err(HostError::Unsupported) }
}

/// Adapts any `Write` (stdout, `Vec<u8>`, `sink()`) into a `Host`.
#[derive(Clone, Debug)]
pub struct WriterHost<W: Write>(pub W);

impl<W: Write> Host for WriterHost<W> {
    fn write_line(&mut self, text: &str) -> Result<(), HostError> {
        writeln!(self.0, "{text}").map_err(|_err| HostError::Io)
    }
}

/// Deterministic host for tests: captures output, replays canned input, fixed
/// clock.
#[derive(Clone, Debug, Default)]
pub struct TestHost {
    pub output: Vec<String>,
    pub input: Vec<String>,
    pub now: i64,
}

impl Host for TestHost {
    fn write_line(&mut self, text: &str) -> Result<(), HostError> {
        self.output.push(text.into());
        Ok(())
    }

    fn read_line(&mut self) -> Result<Option<String>, HostError> {
        Ok(if self.input.is_empty() { None } else { Some(self.input.remove(0)) })
    }

    fn now_millis(&self) -> Result<i64, HostError> { Ok(self.now) }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;

    use super::*;

    /// Implements only the required method, so the defaults are exercised.
    struct OutputOnly;

    impl Host for OutputOnly {
        fn write_line(&mut self, _text: &str) -> Result<(), HostError> { Ok(()) }
    }

    #[test]
    fn defaults_report_unsupported() {
        let mut host = OutputOnly;
        assert_eq!(host.read_line(), Err(HostError::Unsupported));
        assert_eq!(host.now_millis(), Err(HostError::Unsupported));
    }

    #[test]
    fn test_host_captures_output_in_order() {
        let mut host = TestHost::default();
        host.write_line("a").unwrap();
        host.write_line("b").unwrap();
        assert_eq!(host.output, ["a", "b"]);
    }

    #[test]
    fn test_host_input_is_consumed_front_to_back() {
        let mut host =
            TestHost { input: Vec::from(["x".into(), "y".into()]), ..TestHost::default() };
        assert_eq!(host.read_line(), Ok(Some("x".into())));
        assert_eq!(host.read_line(), Ok(Some("y".into())));
        assert_eq!(host.read_line(), Ok(None));
        // End of input is sticky.
        assert_eq!(host.read_line(), Ok(None));
    }

    #[test]
    fn test_host_clock_is_fixed() {
        let host = TestHost { now: 42, ..TestHost::default() };
        assert_eq!(host.now_millis(), Ok(42));
        assert_eq!(host.now_millis(), Ok(42));
    }

    #[test]
    fn writer_host_appends_a_newline_per_call() {
        let mut host = WriterHost(Vec::<u8>::new());
        host.write_line("hi").unwrap();
        host.write_line("there").unwrap();
        assert_eq!(host.0, b"hi\nthere\n");
    }

    #[test]
    fn writer_host_only_implements_output() {
        let mut host = WriterHost(Vec::<u8>::new());
        assert_eq!(host.read_line(), Err(HostError::Unsupported));
        assert_eq!(host.now_millis(), Err(HostError::Unsupported));
    }

    #[test]
    fn host_error_messages() {
        assert_eq!(HostError::Unsupported.to_string(), "operation not supported by host");
        assert_eq!(HostError::Io.to_string(), "host I/O error");
    }
}
