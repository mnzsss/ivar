use crate::providers::capture::{CaptureEvent, ParseSkip, parse_wire};

pub(crate) fn parse(stdin: &str) -> Result<CaptureEvent, ParseSkip> {
    parse_wire(stdin)
}
